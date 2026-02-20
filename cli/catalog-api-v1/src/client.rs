#[allow(unused_imports)]
pub use progenitor_client::{ByteStream, ClientInfo, Error, ResponseValue};
#[allow(unused_imports)]
use progenitor_client::{encode_path, ClientHooks, OperationInfo, RequestBuilderExt};
/// Types used as operation parameters and responses.
#[allow(clippy::all)]
pub mod types {
    /// Error types.
    pub mod error {
        /// Error from a `TryFrom` or `FromStr` implementation.
        pub struct ConversionError(::std::borrow::Cow<'static, str>);
        impl ::std::error::Error for ConversionError {}
        impl ::std::fmt::Display for ConversionError {
            fn fmt(
                &self,
                f: &mut ::std::fmt::Formatter<'_>,
            ) -> Result<(), ::std::fmt::Error> {
                ::std::fmt::Display::fmt(&self.0, f)
            }
        }
        impl ::std::fmt::Debug for ConversionError {
            fn fmt(
                &self,
                f: &mut ::std::fmt::Formatter<'_>,
            ) -> Result<(), ::std::fmt::Error> {
                ::std::fmt::Debug::fmt(&self.0, f)
            }
        }
        impl From<&'static str> for ConversionError {
            fn from(value: &'static str) -> Self {
                Self(value.into())
            }
        }
        impl From<String> for ConversionError {
            fn from(value: String) -> Self {
                Self(value.into())
            }
        }
    }
    ///`BaseCatalogInfo`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "BaseCatalogInfo",
    ///  "type": "object",
    ///  "required": [
    ///    "base_url",
    ///    "scraped_pages",
    ///    "stabilities"
    ///  ],
    ///  "properties": {
    ///    "base_url": {
    ///      "title": "Base Url",
    ///      "type": "string"
    ///    },
    ///    "scraped_pages": {
    ///      "title": "Scraped Pages",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/PageInfo"
    ///      }
    ///    },
    ///    "stabilities": {
    ///      "title": "Stabilities",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/StabilityInfo"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct BaseCatalogInfo {
        pub base_url: ::std::string::String,
        pub scraped_pages: ::std::vec::Vec<PageInfo>,
        pub stabilities: ::std::vec::Vec<StabilityInfo>,
    }
    impl ::std::convert::From<&BaseCatalogInfo> for BaseCatalogInfo {
        fn from(value: &BaseCatalogInfo) -> Self {
            value.clone()
        }
    }
    ///`CatalogName`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "Catalog Name",
    ///  "examples": [
    ///    "mycatalog"
    ///  ],
    ///  "type": "string",
    ///  "pattern": "[a-zA-Z0-9\\-_]{3,64}"
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Serialize, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    #[serde(transparent)]
    pub struct CatalogName(::std::string::String);
    impl ::std::ops::Deref for CatalogName {
        type Target = ::std::string::String;
        fn deref(&self) -> &::std::string::String {
            &self.0
        }
    }
    impl ::std::convert::From<CatalogName> for ::std::string::String {
        fn from(value: CatalogName) -> Self {
            value.0
        }
    }
    impl ::std::convert::From<&CatalogName> for CatalogName {
        fn from(value: &CatalogName) -> Self {
            value.clone()
        }
    }
    impl ::std::str::FromStr for CatalogName {
        type Err = self::error::ConversionError;
        fn from_str(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            static PATTERN: ::std::sync::LazyLock<::regress::Regex> = ::std::sync::LazyLock::new(||
            { ::regress::Regex::new("[a-zA-Z0-9\\-_]{3,64}").unwrap() });
            if PATTERN.find(value).is_none() {
                return Err("doesn't match pattern \"[a-zA-Z0-9\\-_]{3,64}\"".into());
            }
            Ok(Self(value.to_string()))
        }
    }
    impl ::std::convert::TryFrom<&str> for CatalogName {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<&::std::string::String> for CatalogName {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<::std::string::String> for CatalogName {
        type Error = self::error::ConversionError;
        fn try_from(
            value: ::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl<'de> ::serde::Deserialize<'de> for CatalogName {
        fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
        where
            D: ::serde::Deserializer<'de>,
        {
            ::std::string::String::deserialize(deserializer)?
                .parse()
                .map_err(|e: self::error::ConversionError| {
                    <D::Error as ::serde::de::Error>::custom(e.to_string())
                })
        }
    }
    ///`CatalogPage`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "CatalogPage",
    ///  "examples": [
    ///    {
    ///      "attr_path": "curl",
    ///      "broken": false,
    ///      "catalog": "nixpkgs",
    ///      "derivation": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-curl-8.5.0.drv",
    ///      "description": "A command line tool for transferring files with URL syntax",
    ///      "insecure": false,
    ///      "license": "curl",
    ///      "locked_url": "https://github.com/flox/nixpkgs?rev=abc123def456",
    ///      "missing_builds": false,
    ///      "name": "curl-8.5.0",
    ///      "outputs": [
    ///        {
    ///          "name": "out",
    ///          "store_path": "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-curl-8.5.0"
    ///        },
    ///        {
    ///          "name": "man",
    ///          "store_path": "/nix/store/cccccccccccccccccccccccccccccccc-curl-8.5.0-man"
    ///        }
    ///      ],
    ///      "outputs_to_install": [
    ///        "out",
    ///        "man"
    ///      ],
    ///      "pkg_path": "curl",
    ///      "pname": "curl",
    ///      "rev": "abc123def456",
    ///      "rev_count": 12345,
    ///      "rev_date": "2024-01-15T00:00:00Z",
    ///      "stabilities": [
    ///        "stable"
    ///      ],
    ///      "system": "x86_64-linux",
    ///      "unfree": false,
    ///      "version": "8.5.0"
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "complete",
    ///    "messages",
    ///    "page",
    ///    "url"
    ///  ],
    ///  "properties": {
    ///    "complete": {
    ///      "title": "Complete",
    ///      "type": "boolean"
    ///    },
    ///    "messages": {
    ///      "title": "Messages",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/ResolutionMessageGeneral"
    ///      }
    ///    },
    ///    "packages": {
    ///      "title": "Packages",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "$ref": "#/components/schemas/ResolvedPackageDescriptor"
    ///      }
    ///    },
    ///    "page": {
    ///      "title": "Page",
    ///      "type": "integer"
    ///    },
    ///    "url": {
    ///      "title": "Url",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct CatalogPage {
        pub complete: bool,
        pub messages: ::std::vec::Vec<ResolutionMessageGeneral>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub packages: ::std::option::Option<::std::vec::Vec<ResolvedPackageDescriptor>>,
        pub page: i64,
        pub url: ::std::string::String,
    }
    impl ::std::convert::From<&CatalogPage> for CatalogPage {
        fn from(value: &CatalogPage) -> Self {
            value.clone()
        }
    }
    ///`CatalogShareInfo`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "CatalogShareInfo",
    ///  "type": "object",
    ///  "properties": {
    ///    "allow_read_users": {
    ///      "title": "Allow Read Users",
    ///      "default": [],
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct CatalogShareInfo {
        #[serde(default = "defaults::catalog_share_info_allow_read_users")]
        pub allow_read_users: ::std::option::Option<
            ::std::vec::Vec<::std::string::String>,
        >,
    }
    impl ::std::convert::From<&CatalogShareInfo> for CatalogShareInfo {
        fn from(value: &CatalogShareInfo) -> Self {
            value.clone()
        }
    }
    impl ::std::default::Default for CatalogShareInfo {
        fn default() -> Self {
            Self {
                allow_read_users: defaults::catalog_share_info_allow_read_users(),
            }
        }
    }
    ///`CatalogStatus`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "CatalogStatus",
    ///  "type": "object",
    ///  "required": [
    ///    "attribute_path_ct",
    ///    "derivations_ct",
    ///    "latest_rev",
    ///    "pages_ct",
    ///    "schema_version",
    ///    "search_index_ct",
    ///    "systems",
    ///    "tags"
    ///  ],
    ///  "properties": {
    ///    "attribute_path_ct": {
    ///      "title": "Attribute Path Ct",
    ///      "type": "integer"
    ///    },
    ///    "derivations_ct": {
    ///      "title": "Derivations Ct",
    ///      "type": "integer"
    ///    },
    ///    "latest_rev": {
    ///      "title": "Latest Rev",
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "latest_scrape": {
    ///      "title": "Latest Scrape",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ],
    ///      "format": "date-time"
    ///    },
    ///    "narinfos_ct": {
    ///      "title": "Narinfos Ct",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "pages_ct": {
    ///      "title": "Pages Ct",
    ///      "type": "integer"
    ///    },
    ///    "schema_version": {
    ///      "title": "Schema Version",
    ///      "type": "string"
    ///    },
    ///    "search_index_ct": {
    ///      "title": "Search Index Ct",
    ///      "type": "integer"
    ///    },
    ///    "storepaths_ct": {
    ///      "title": "Storepaths Ct",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "systems": {
    ///      "title": "Systems",
    ///      "type": "array",
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "tags": {
    ///      "title": "Tags",
    ///      "type": "object",
    ///      "additionalProperties": {
    ///        "type": "array",
    ///        "items": {
    ///          "type": "string"
    ///        }
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct CatalogStatus {
        pub attribute_path_ct: i64,
        pub derivations_ct: i64,
        pub latest_rev: ::chrono::DateTime<::chrono::offset::Utc>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub latest_scrape: ::std::option::Option<
            ::chrono::DateTime<::chrono::offset::Utc>,
        >,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub narinfos_ct: ::std::option::Option<i64>,
        pub pages_ct: i64,
        pub schema_version: ::std::string::String,
        pub search_index_ct: i64,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub storepaths_ct: ::std::option::Option<i64>,
        pub systems: ::std::vec::Vec<::std::string::String>,
        pub tags: ::std::collections::HashMap<
            ::std::string::String,
            ::std::vec::Vec<::std::string::String>,
        >,
    }
    impl ::std::convert::From<&CatalogStatus> for CatalogStatus {
        fn from(value: &CatalogStatus) -> Self {
            value.clone()
        }
    }
    ///`CatalogStoreConfigMetaOnly`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "CatalogStoreConfigMetaOnly",
    ///  "type": "object",
    ///  "properties": {
    ///    "store_type": {
    ///      "title": "Store Type",
    ///      "default": "meta-only",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct CatalogStoreConfigMetaOnly {
        #[serde(default = "defaults::catalog_store_config_meta_only_store_type")]
        pub store_type: ::std::string::String,
    }
    impl ::std::convert::From<&CatalogStoreConfigMetaOnly>
    for CatalogStoreConfigMetaOnly {
        fn from(value: &CatalogStoreConfigMetaOnly) -> Self {
            value.clone()
        }
    }
    impl ::std::default::Default for CatalogStoreConfigMetaOnly {
        fn default() -> Self {
            Self {
                store_type: defaults::catalog_store_config_meta_only_store_type(),
            }
        }
    }
    ///`CatalogStoreConfigNixCopy`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "CatalogStoreConfigNixCopy",
    ///  "type": "object",
    ///  "required": [
    ///    "egress_uri",
    ///    "ingress_uri"
    ///  ],
    ///  "properties": {
    ///    "egress_uri": {
    ///      "title": "Egress Uri",
    ///      "type": "string"
    ///    },
    ///    "ingress_uri": {
    ///      "title": "Ingress Uri",
    ///      "type": "string"
    ///    },
    ///    "store_type": {
    ///      "title": "Store Type",
    ///      "default": "nix-copy",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct CatalogStoreConfigNixCopy {
        pub egress_uri: ::std::string::String,
        pub ingress_uri: ::std::string::String,
        #[serde(default = "defaults::catalog_store_config_nix_copy_store_type")]
        pub store_type: ::std::string::String,
    }
    impl ::std::convert::From<&CatalogStoreConfigNixCopy> for CatalogStoreConfigNixCopy {
        fn from(value: &CatalogStoreConfigNixCopy) -> Self {
            value.clone()
        }
    }
    ///`CatalogStoreConfigNull`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "CatalogStoreConfigNull",
    ///  "type": "object",
    ///  "properties": {
    ///    "store_type": {
    ///      "title": "Store Type",
    ///      "default": "null",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct CatalogStoreConfigNull {
        #[serde(default = "defaults::catalog_store_config_null_store_type")]
        pub store_type: ::std::string::String,
    }
    impl ::std::convert::From<&CatalogStoreConfigNull> for CatalogStoreConfigNull {
        fn from(value: &CatalogStoreConfigNull) -> Self {
            value.clone()
        }
    }
    impl ::std::default::Default for CatalogStoreConfigNull {
        fn default() -> Self {
            Self {
                store_type: defaults::catalog_store_config_null_store_type(),
            }
        }
    }
    ///`CatalogStoreConfigPublisher`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "CatalogStoreConfigPublisher",
    ///  "type": "object",
    ///  "properties": {
    ///    "publisher_url": {
    ///      "title": "Publisher Url",
    ///      "deprecated": true,
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "store_type": {
    ///      "title": "Store Type",
    ///      "default": "publisher",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct CatalogStoreConfigPublisher {
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub publisher_url: ::std::option::Option<::std::string::String>,
        #[serde(default = "defaults::catalog_store_config_publisher_store_type")]
        pub store_type: ::std::string::String,
    }
    impl ::std::convert::From<&CatalogStoreConfigPublisher>
    for CatalogStoreConfigPublisher {
        fn from(value: &CatalogStoreConfigPublisher) -> Self {
            value.clone()
        }
    }
    impl ::std::default::Default for CatalogStoreConfigPublisher {
        fn default() -> Self {
            Self {
                publisher_url: Default::default(),
                store_type: defaults::catalog_store_config_publisher_store_type(),
            }
        }
    }
    /**Request body for environment SBOM endpoint.

The environment SBOM endpoint generates a Software Bill of Materials
for a Flox environment by analyzing its lockfile and dependencies.

Attributes:
    lockfile: The environment's lockfile dictionary (v0 or v1 format)
    system: Target system architecture (e.g., "x86_64-linux", "aarch64-darwin")
    environment_name: Name of the environment (for informational purposes in SBOM)
    environment_owner: Owner of the environment (for informational purposes in SBOM)
    generation: Optional generation number (for informational purposes in SBOM)*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "EnvironmentSbomRequest",
    ///  "description": "Request body for environment SBOM endpoint.\n\nThe environment SBOM endpoint generates a Software Bill of Materials\nfor a Flox environment by analyzing its lockfile and dependencies.\n\nAttributes:\n    lockfile: The environment's lockfile dictionary (v0 or v1 format)\n    system: Target system architecture (e.g., \"x86_64-linux\", \"aarch64-darwin\")\n    environment_name: Name of the environment (for informational purposes in SBOM)\n    environment_owner: Owner of the environment (for informational purposes in SBOM)\n    generation: Optional generation number (for informational purposes in SBOM)",
    ///  "type": "object",
    ///  "required": [
    ///    "environment_name",
    ///    "environment_owner",
    ///    "lockfile",
    ///    "system"
    ///  ],
    ///  "properties": {
    ///    "environment_name": {
    ///      "title": "Environment Name",
    ///      "type": "string"
    ///    },
    ///    "environment_owner": {
    ///      "title": "Environment Owner",
    ///      "type": "string"
    ///    },
    ///    "generation": {
    ///      "title": "Generation",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "lockfile": {
    ///      "title": "Lockfile",
    ///      "type": "object",
    ///      "additionalProperties": true
    ///    },
    ///    "system": {
    ///      "$ref": "#/components/schemas/PackageSystem"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct EnvironmentSbomRequest {
        pub environment_name: ::std::string::String,
        pub environment_owner: ::std::string::String,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub generation: ::std::option::Option<i64>,
        pub lockfile: ::serde_json::Map<::std::string::String, ::serde_json::Value>,
        pub system: PackageSystem,
    }
    impl ::std::convert::From<&EnvironmentSbomRequest> for EnvironmentSbomRequest {
        fn from(value: &EnvironmentSbomRequest) -> Self {
            value.clone()
        }
    }
    ///`ErrorResponse`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ErrorResponse",
    ///  "type": "object",
    ///  "required": [
    ///    "detail"
    ///  ],
    ///  "properties": {
    ///    "detail": {
    ///      "title": "Detail",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct ErrorResponse {
        pub detail: ::std::string::String,
    }
    impl ::std::convert::From<&ErrorResponse> for ErrorResponse {
        fn from(value: &ErrorResponse) -> Self {
            value.clone()
        }
    }
    ///`HealthCheck`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "HealthCheck",
    ///  "type": "object",
    ///  "required": [
    ///    "resolve_elapsed_ms",
    ///    "resolve_ok",
    ///    "search_elapsed_ms",
    ///    "search_ok",
    ///    "show_elapsed_ms",
    ///    "show_ok"
    ///  ],
    ///  "properties": {
    ///    "check_parameters": {
    ///      "$ref": "#/components/schemas/params"
    ///    },
    ///    "resolve_elapsed_ms": {
    ///      "title": "Resolve Elapsed Ms",
    ///      "type": "integer"
    ///    },
    ///    "resolve_ok": {
    ///      "title": "Resolve Ok",
    ///      "type": "boolean"
    ///    },
    ///    "search_elapsed_ms": {
    ///      "title": "Search Elapsed Ms",
    ///      "type": "integer"
    ///    },
    ///    "search_ok": {
    ///      "title": "Search Ok",
    ///      "type": "boolean"
    ///    },
    ///    "show_elapsed_ms": {
    ///      "title": "Show Elapsed Ms",
    ///      "type": "integer"
    ///    },
    ///    "show_ok": {
    ///      "title": "Show Ok",
    ///      "type": "boolean"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct HealthCheck {
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub check_parameters: ::std::option::Option<Params>,
        pub resolve_elapsed_ms: i64,
        pub resolve_ok: bool,
        pub search_elapsed_ms: i64,
        pub search_ok: bool,
        pub show_elapsed_ms: i64,
        pub show_ok: bool,
    }
    impl ::std::convert::From<&HealthCheck> for HealthCheck {
        fn from(value: &HealthCheck) -> Self {
            value.clone()
        }
    }
    ///`MessageLevel`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "MessageLevel",
    ///  "type": "string",
    ///  "enum": [
    ///    "trace",
    ///    "info",
    ///    "warning",
    ///    "error"
    ///  ]
    ///}
    /// ```
    /// </details>
    #[derive(
        ::serde::Deserialize,
        ::serde::Serialize,
        Clone,
        Copy,
        Debug,
        Eq,
        Hash,
        Ord,
        PartialEq,
        PartialOrd
    )]
    pub enum MessageLevel {
        #[serde(rename = "trace")]
        Trace,
        #[serde(rename = "info")]
        Info,
        #[serde(rename = "warning")]
        Warning,
        #[serde(rename = "error")]
        Error,
    }
    impl ::std::convert::From<&Self> for MessageLevel {
        fn from(value: &MessageLevel) -> Self {
            value.clone()
        }
    }
    impl ::std::fmt::Display for MessageLevel {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
            match *self {
                Self::Trace => f.write_str("trace"),
                Self::Info => f.write_str("info"),
                Self::Warning => f.write_str("warning"),
                Self::Error => f.write_str("error"),
            }
        }
    }
    impl ::std::str::FromStr for MessageLevel {
        type Err = self::error::ConversionError;
        fn from_str(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            match value {
                "trace" => Ok(Self::Trace),
                "info" => Ok(Self::Info),
                "warning" => Ok(Self::Warning),
                "error" => Ok(Self::Error),
                _ => Err("invalid value".into()),
            }
        }
    }
    impl ::std::convert::TryFrom<&str> for MessageLevel {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<&::std::string::String> for MessageLevel {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<::std::string::String> for MessageLevel {
        type Error = self::error::ConversionError;
        fn try_from(
            value: ::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    ///`NarFileLookup`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "NarFileLookup",
    ///  "type": "object",
    ///  "required": [
    ///    "derivations",
    ///    "storepath"
    ///  ],
    ///  "properties": {
    ///    "derivations": {
    ///      "title": "Derivations",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/PackageDerivation-Output"
    ///      }
    ///    },
    ///    "storepath": {
    ///      "title": "Storepath",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct NarFileLookup {
        pub derivations: ::std::vec::Vec<PackageDerivationOutput>,
        pub storepath: ::std::string::String,
    }
    impl ::std::convert::From<&NarFileLookup> for NarFileLookup {
        fn from(value: &NarFileLookup) -> Self {
            value.clone()
        }
    }
    ///`NarInfo`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "NarInfo",
    ///  "type": "object",
    ///  "properties": {
    ///    "ca": {
    ///      "title": "Ca",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "closureDownloadSize": {
    ///      "title": "Closuredownloadsize",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "closureSize": {
    ///      "title": "Closuresize",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "compression": {
    ///      "title": "Compression",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "compresssize": {
    ///      "title": "Compresssize",
    ///      "deprecated": true,
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "deriver": {
    ///      "title": "Deriver",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "downloadHash": {
    ///      "title": "Downloadhash",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "downloadSize": {
    ///      "title": "Downloadsize",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "narHash": {
    ///      "title": "Narhash",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "narSize": {
    ///      "title": "Narsize",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "path": {
    ///      "title": "Path",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "references": {
    ///      "title": "References",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "registrationTime": {
    ///      "title": "Registrationtime",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "signatures": {
    ///      "title": "Signatures",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "ultimate": {
    ///      "title": "Ultimate",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "url": {
    ///      "title": "Url",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct NarInfo {
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub ca: ::std::option::Option<::std::string::String>,
        #[serde(
            rename = "closureDownloadSize",
            default,
            skip_serializing_if = "::std::option::Option::is_none"
        )]
        pub closure_download_size: ::std::option::Option<i64>,
        #[serde(
            rename = "closureSize",
            default,
            skip_serializing_if = "::std::option::Option::is_none"
        )]
        pub closure_size: ::std::option::Option<i64>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub compression: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub compresssize: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub deriver: ::std::option::Option<::std::string::String>,
        #[serde(
            rename = "downloadHash",
            default,
            skip_serializing_if = "::std::option::Option::is_none"
        )]
        pub download_hash: ::std::option::Option<::std::string::String>,
        #[serde(
            rename = "downloadSize",
            default,
            skip_serializing_if = "::std::option::Option::is_none"
        )]
        pub download_size: ::std::option::Option<i64>,
        #[serde(
            rename = "narHash",
            default,
            skip_serializing_if = "::std::option::Option::is_none"
        )]
        pub nar_hash: ::std::option::Option<::std::string::String>,
        #[serde(
            rename = "narSize",
            default,
            skip_serializing_if = "::std::option::Option::is_none"
        )]
        pub nar_size: ::std::option::Option<i64>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub path: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub references: ::std::option::Option<::std::vec::Vec<::std::string::String>>,
        #[serde(
            rename = "registrationTime",
            default,
            skip_serializing_if = "::std::option::Option::is_none"
        )]
        pub registration_time: ::std::option::Option<i64>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub signatures: ::std::option::Option<::std::vec::Vec<::std::string::String>>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub ultimate: ::std::option::Option<bool>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub url: ::std::option::Option<::std::string::String>,
    }
    impl ::std::convert::From<&NarInfo> for NarInfo {
        fn from(value: &NarInfo) -> Self {
            value.clone()
        }
    }
    impl ::std::default::Default for NarInfo {
        fn default() -> Self {
            Self {
                ca: Default::default(),
                closure_download_size: Default::default(),
                closure_size: Default::default(),
                compression: Default::default(),
                compresssize: Default::default(),
                deriver: Default::default(),
                download_hash: Default::default(),
                download_size: Default::default(),
                nar_hash: Default::default(),
                nar_size: Default::default(),
                path: Default::default(),
                references: Default::default(),
                registration_time: Default::default(),
                signatures: Default::default(),
                ultimate: Default::default(),
                url: Default::default(),
            }
        }
    }
    ///`NarInfos`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "NarInfos",
    ///  "type": "object",
    ///  "additionalProperties": {
    ///    "$ref": "#/components/schemas/NarInfo"
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    #[serde(transparent)]
    pub struct NarInfos(pub ::std::collections::HashMap<::std::string::String, NarInfo>);
    impl ::std::ops::Deref for NarInfos {
        type Target = ::std::collections::HashMap<::std::string::String, NarInfo>;
        fn deref(&self) -> &::std::collections::HashMap<::std::string::String, NarInfo> {
            &self.0
        }
    }
    impl ::std::convert::From<NarInfos>
    for ::std::collections::HashMap<::std::string::String, NarInfo> {
        fn from(value: NarInfos) -> Self {
            value.0
        }
    }
    impl ::std::convert::From<&NarInfos> for NarInfos {
        fn from(value: &NarInfos) -> Self {
            value.clone()
        }
    }
    impl ::std::convert::From<
        ::std::collections::HashMap<::std::string::String, NarInfo>,
    > for NarInfos {
        fn from(
            value: ::std::collections::HashMap<::std::string::String, NarInfo>,
        ) -> Self {
            Self(value)
        }
    }
    ///Comma-separated list of output names (e.g., 'out,bin,dev')
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "Outputs",
    ///  "description": "Comma-separated list of output names (e.g., 'out,bin,dev')",
    ///  "examples": [
    ///    "out"
    ///  ],
    ///  "type": "string",
    ///  "maxLength": 100,
    ///  "pattern": "^[a-zA-Z0-9_]+(?:,[a-zA-Z0-9_]+)*$"
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Serialize, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    #[serde(transparent)]
    pub struct Outputs(::std::string::String);
    impl ::std::ops::Deref for Outputs {
        type Target = ::std::string::String;
        fn deref(&self) -> &::std::string::String {
            &self.0
        }
    }
    impl ::std::convert::From<Outputs> for ::std::string::String {
        fn from(value: Outputs) -> Self {
            value.0
        }
    }
    impl ::std::convert::From<&Outputs> for Outputs {
        fn from(value: &Outputs) -> Self {
            value.clone()
        }
    }
    impl ::std::str::FromStr for Outputs {
        type Err = self::error::ConversionError;
        fn from_str(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            if value.chars().count() > 100usize {
                return Err("longer than 100 characters".into());
            }
            static PATTERN: ::std::sync::LazyLock<::regress::Regex> = ::std::sync::LazyLock::new(||
            { ::regress::Regex::new("^[a-zA-Z0-9_]+(?:,[a-zA-Z0-9_]+)*$").unwrap() });
            if PATTERN.find(value).is_none() {
                return Err(
                    "doesn't match pattern \"^[a-zA-Z0-9_]+(?:,[a-zA-Z0-9_]+)*$\"".into(),
                );
            }
            Ok(Self(value.to_string()))
        }
    }
    impl ::std::convert::TryFrom<&str> for Outputs {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<&::std::string::String> for Outputs {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<::std::string::String> for Outputs {
        type Error = self::error::ConversionError;
        fn try_from(
            value: ::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl<'de> ::serde::Deserialize<'de> for Outputs {
        fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
        where
            D: ::serde::Deserializer<'de>,
        {
            ::std::string::String::deserialize(deserializer)?
                .parse()
                .map_err(|e: self::error::ConversionError| {
                    <D::Error as ::serde::de::Error>::custom(e.to_string())
                })
        }
    }
    ///`PackageBuild`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageBuild",
    ///  "examples": [
    ///    {
    ///      "derivation": {
    ///        "broken": false,
    ///        "description": "A command line tool for transferring files with URL syntax",
    ///        "drv_path": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-curl-8.5.0.drv",
    ///        "license": "curl",
    ///        "name": "curl-8.5.0",
    ///        "outputs": {
    ///          "man": "/nix/store/cccccccccccccccccccccccccccccccc-curl-8.5.0-man",
    ///          "out": "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-curl-8.5.0"
    ///        },
    ///        "outputs_to_install": [
    ///          "out",
    ///          "man"
    ///        ],
    ///        "pname": "curl",
    ///        "system": "x86_64-linux",
    ///        "unfree": false,
    ///        "version": "8.5.0"
    ///      },
    ///      "locked_base_catalog_url": "https://github.com/flox/nixpkgs?rev=99dc8785f6a0adac95f5e2ab05cc2e1bf666d172",
    ///      "rev": "99dc8785f6a0adac95f5e2ab05cc2e1bf666d172",
    ///      "rev_count": 12345,
    ///      "rev_date": "2021-09-01T00:00:00Z",
    ///      "url": "https://github.com/org/example"
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "derivation",
    ///    "rev",
    ///    "rev_count",
    ///    "rev_date",
    ///    "url"
    ///  ],
    ///  "properties": {
    ///    "cache_uri": {
    ///      "title": "Cache Uri",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "derivation": {
    ///      "$ref": "#/components/schemas/PackageDerivation-Output"
    ///    },
    ///    "locked_base_catalog_url": {
    ///      "title": "Locked Base Catalog Url",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "rev": {
    ///      "title": "Rev",
    ///      "type": "string"
    ///    },
    ///    "rev_count": {
    ///      "title": "Rev Count",
    ///      "type": "integer"
    ///    },
    ///    "rev_date": {
    ///      "title": "Rev Date",
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "url": {
    ///      "title": "Url",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackageBuild {
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub cache_uri: ::std::option::Option<::std::string::String>,
        pub derivation: PackageDerivationOutput,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub locked_base_catalog_url: ::std::option::Option<::std::string::String>,
        pub rev: ::std::string::String,
        pub rev_count: i64,
        pub rev_date: ::chrono::DateTime<::chrono::offset::Utc>,
        pub url: ::std::string::String,
    }
    impl ::std::convert::From<&PackageBuild> for PackageBuild {
        fn from(value: &PackageBuild) -> Self {
            value.clone()
        }
    }
    ///`PackageBuildList`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageBuildList",
    ///  "type": "object",
    ///  "required": [
    ///    "items"
    ///  ],
    ///  "properties": {
    ///    "items": {
    ///      "title": "Items",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/PackageBuild"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackageBuildList {
        pub items: ::std::vec::Vec<PackageBuild>,
    }
    impl ::std::convert::From<&PackageBuildList> for PackageBuildList {
        fn from(value: &PackageBuildList) -> Self {
            value.clone()
        }
    }
    ///`PackageBuildResponse`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageBuildResponse",
    ///  "type": "object"
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    #[serde(transparent)]
    pub struct PackageBuildResponse(
        pub ::serde_json::Map<::std::string::String, ::serde_json::Value>,
    );
    impl ::std::ops::Deref for PackageBuildResponse {
        type Target = ::serde_json::Map<::std::string::String, ::serde_json::Value>;
        fn deref(
            &self,
        ) -> &::serde_json::Map<::std::string::String, ::serde_json::Value> {
            &self.0
        }
    }
    impl ::std::convert::From<PackageBuildResponse>
    for ::serde_json::Map<::std::string::String, ::serde_json::Value> {
        fn from(value: PackageBuildResponse) -> Self {
            value.0
        }
    }
    impl ::std::convert::From<&PackageBuildResponse> for PackageBuildResponse {
        fn from(value: &PackageBuildResponse) -> Self {
            value.clone()
        }
    }
    impl ::std::convert::From<
        ::serde_json::Map<::std::string::String, ::serde_json::Value>,
    > for PackageBuildResponse {
        fn from(
            value: ::serde_json::Map<::std::string::String, ::serde_json::Value>,
        ) -> Self {
            Self(value)
        }
    }
    ///`PackageBuildWithNarInfo`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageBuildWithNarInfo",
    ///  "examples": [
    ///    {
    ///      "derivation": {
    ///        "broken": false,
    ///        "description": "A command line tool for transferring files with URL syntax",
    ///        "drv_path": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-curl-8.5.0.drv",
    ///        "license": "curl",
    ///        "name": "curl-8.5.0",
    ///        "outputs": {
    ///          "man": "/nix/store/cccccccccccccccccccccccccccccccc-curl-8.5.0-man",
    ///          "out": "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-curl-8.5.0"
    ///        },
    ///        "outputs_to_install": [
    ///          "out",
    ///          "man"
    ///        ],
    ///        "pname": "curl",
    ///        "system": "x86_64-linux",
    ///        "unfree": false,
    ///        "version": "8.5.0"
    ///      },
    ///      "locked_base_catalog_url": "https://github.com/flox/nixpkgs?rev=99dc8785f6a0adac95f5e2ab05cc2e1bf666d172",
    ///      "rev": "99dc8785f6a0adac95f5e2ab05cc2e1bf666d172",
    ///      "rev_count": 12345,
    ///      "rev_date": "2021-09-01T00:00:00Z",
    ///      "url": "https://github.com/org/example"
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "derivation",
    ///    "rev",
    ///    "rev_count",
    ///    "rev_date",
    ///    "url"
    ///  ],
    ///  "properties": {
    ///    "cache_uri": {
    ///      "title": "Cache Uri",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "derivation": {
    ///      "$ref": "#/components/schemas/PackageDerivation-Input"
    ///    },
    ///    "locked_base_catalog_url": {
    ///      "title": "Locked Base Catalog Url",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "narinfos": {
    ///      "$ref": "#/components/schemas/NarInfos"
    ///    },
    ///    "narinfos_source_url": {
    ///      "title": "Narinfos Source Url",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "narinfos_source_version": {
    ///      "title": "Narinfos Source Version",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "rev": {
    ///      "title": "Rev",
    ///      "type": "string"
    ///    },
    ///    "rev_count": {
    ///      "title": "Rev Count",
    ///      "type": "integer"
    ///    },
    ///    "rev_date": {
    ///      "title": "Rev Date",
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "url": {
    ///      "title": "Url",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackageBuildWithNarInfo {
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub cache_uri: ::std::option::Option<::std::string::String>,
        pub derivation: PackageDerivationInput,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub locked_base_catalog_url: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub narinfos: ::std::option::Option<NarInfos>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub narinfos_source_url: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub narinfos_source_version: ::std::option::Option<i64>,
        pub rev: ::std::string::String,
        pub rev_count: i64,
        pub rev_date: ::chrono::DateTime<::chrono::offset::Utc>,
        pub url: ::std::string::String,
    }
    impl ::std::convert::From<&PackageBuildWithNarInfo> for PackageBuildWithNarInfo {
        fn from(value: &PackageBuildWithNarInfo) -> Self {
            value.clone()
        }
    }
    ///`PackageDerivationInput`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageDerivation",
    ///  "examples": [
    ///    {
    ///      "broken": false,
    ///      "description": "A command line tool for transferring files with URL syntax",
    ///      "drv_path": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-curl-8.5.0.drv",
    ///      "license": "curl",
    ///      "name": "curl-8.5.0",
    ///      "outputs": {
    ///        "man": "/nix/store/cccccccccccccccccccccccccccccccc-curl-8.5.0-man",
    ///        "out": "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-curl-8.5.0"
    ///      },
    ///      "outputs_to_install": [
    ///        "out",
    ///        "man"
    ///      ],
    ///      "pname": "curl",
    ///      "system": "x86_64-linux",
    ///      "unfree": false,
    ///      "version": "8.5.0"
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "drv_path",
    ///    "name",
    ///    "outputs",
    ///    "system"
    ///  ],
    ///  "properties": {
    ///    "broken": {
    ///      "title": "Broken",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "description": {
    ///      "title": "Description",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "drv_path": {
    ///      "title": "Drv Path",
    ///      "type": "string"
    ///    },
    ///    "license": {
    ///      "title": "License",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "outputs": {
    ///      "$ref": "#/components/schemas/PackageOutputs"
    ///    },
    ///    "outputs_to_install": {
    ///      "title": "Outputs To Install",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "pname": {
    ///      "title": "Pname",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "system": {
    ///      "$ref": "#/components/schemas/PackageSystem"
    ///    },
    ///    "unfree": {
    ///      "title": "Unfree",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "version": {
    ///      "title": "Version",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackageDerivationInput {
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub broken: ::std::option::Option<bool>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub description: ::std::option::Option<::std::string::String>,
        pub drv_path: ::std::string::String,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub license: ::std::option::Option<::std::string::String>,
        pub name: ::std::string::String,
        pub outputs: PackageOutputs,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub outputs_to_install: ::std::option::Option<
            ::std::vec::Vec<::std::string::String>,
        >,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub pname: ::std::option::Option<::std::string::String>,
        pub system: PackageSystem,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub unfree: ::std::option::Option<bool>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub version: ::std::option::Option<::std::string::String>,
    }
    impl ::std::convert::From<&PackageDerivationInput> for PackageDerivationInput {
        fn from(value: &PackageDerivationInput) -> Self {
            value.clone()
        }
    }
    ///`PackageDerivationOutput`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageDerivation",
    ///  "examples": [
    ///    {
    ///      "broken": false,
    ///      "description": "A command line tool for transferring files with URL syntax",
    ///      "drv_path": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-curl-8.5.0.drv",
    ///      "license": "curl",
    ///      "name": "curl-8.5.0",
    ///      "outputs": {
    ///        "man": "/nix/store/cccccccccccccccccccccccccccccccc-curl-8.5.0-man",
    ///        "out": "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-curl-8.5.0"
    ///      },
    ///      "outputs_to_install": [
    ///        "out",
    ///        "man"
    ///      ],
    ///      "pname": "curl",
    ///      "system": "x86_64-linux",
    ///      "unfree": false,
    ///      "version": "8.5.0"
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "drv_path",
    ///    "name",
    ///    "outputs",
    ///    "system"
    ///  ],
    ///  "properties": {
    ///    "broken": {
    ///      "title": "Broken",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "description": {
    ///      "title": "Description",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "drv_path": {
    ///      "title": "Drv Path",
    ///      "type": "string"
    ///    },
    ///    "license": {
    ///      "title": "License",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "outputs": {
    ///      "$ref": "#/components/schemas/PackageOutputs"
    ///    },
    ///    "outputs_to_install": {
    ///      "title": "Outputs To Install",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "pname": {
    ///      "title": "Pname",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "system": {
    ///      "$ref": "#/components/schemas/PackageSystem"
    ///    },
    ///    "unfree": {
    ///      "title": "Unfree",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "version": {
    ///      "title": "Version",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackageDerivationOutput {
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub broken: ::std::option::Option<bool>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub description: ::std::option::Option<::std::string::String>,
        pub drv_path: ::std::string::String,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub license: ::std::option::Option<::std::string::String>,
        pub name: ::std::string::String,
        pub outputs: PackageOutputs,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub outputs_to_install: ::std::option::Option<
            ::std::vec::Vec<::std::string::String>,
        >,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub pname: ::std::option::Option<::std::string::String>,
        pub system: PackageSystem,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub unfree: ::std::option::Option<bool>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub version: ::std::option::Option<::std::string::String>,
    }
    impl ::std::convert::From<&PackageDerivationOutput> for PackageDerivationOutput {
        fn from(value: &PackageDerivationOutput) -> Self {
            value.clone()
        }
    }
    ///`PackageDescriptor`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageDescriptor",
    ///  "examples": [
    ///    {
    ///      "attr_path": "curl",
    ///      "install_id": "curl",
    ///      "systems": [
    ///        "x86_64-linux"
    ///      ]
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "attr_path",
    ///    "install_id",
    ///    "systems"
    ///  ],
    ///  "properties": {
    ///    "allow_broken": {
    ///      "title": "Allow Broken",
    ///      "default": false,
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "allow_insecure": {
    ///      "title": "Allow Insecure",
    ///      "default": false,
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "allow_missing_builds": {
    ///      "title": "Allow Missing Builds",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "allow_pre_releases": {
    ///      "title": "Allow Pre Releases",
    ///      "default": false,
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "allow_unfree": {
    ///      "title": "Allow Unfree",
    ///      "default": true,
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "allowed_licenses": {
    ///      "title": "Allowed Licenses",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "attr_path": {
    ///      "title": "Attr Path",
    ///      "type": "string"
    ///    },
    ///    "derivation": {
    ///      "title": "Derivation",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "install_id": {
    ///      "title": "Install Id",
    ///      "type": "string"
    ///    },
    ///    "systems": {
    ///      "title": "Systems",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/PackageSystem"
    ///      }
    ///    },
    ///    "version": {
    ///      "title": "Version",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackageDescriptor {
        #[serde(default = "defaults::package_descriptor_allow_broken")]
        pub allow_broken: ::std::option::Option<bool>,
        #[serde(default = "defaults::package_descriptor_allow_insecure")]
        pub allow_insecure: ::std::option::Option<bool>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub allow_missing_builds: ::std::option::Option<bool>,
        #[serde(default = "defaults::package_descriptor_allow_pre_releases")]
        pub allow_pre_releases: ::std::option::Option<bool>,
        #[serde(default = "defaults::package_descriptor_allow_unfree")]
        pub allow_unfree: ::std::option::Option<bool>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub allowed_licenses: ::std::option::Option<
            ::std::vec::Vec<::std::string::String>,
        >,
        pub attr_path: ::std::string::String,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub derivation: ::std::option::Option<::std::string::String>,
        pub install_id: ::std::string::String,
        pub systems: ::std::vec::Vec<PackageSystem>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub version: ::std::option::Option<::std::string::String>,
    }
    impl ::std::convert::From<&PackageDescriptor> for PackageDescriptor {
        fn from(value: &PackageDescriptor) -> Self {
            value.clone()
        }
    }
    ///`PackageGroup`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageGroup",
    ///  "examples": [
    ///    {
    ///      "descriptors": [
    ///        {
    ///          "attr_path": "curl",
    ///          "install_id": "curl",
    ///          "systems": [
    ///            "x86_64-linux"
    ///          ]
    ///        },
    ///        {
    ///          "attr_path": "slack",
    ///          "install_id": "slack",
    ///          "systems": [
    ///            "x86_64-linux"
    ///          ]
    ///        },
    ///        {
    ///          "attr_path": "xorg.xeyes",
    ///          "install_id": "xeyes",
    ///          "systems": [
    ///            "x86_64-linux"
    ///          ]
    ///        }
    ///      ],
    ///      "name": "test"
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "descriptors",
    ///    "name"
    ///  ],
    ///  "properties": {
    ///    "descriptors": {
    ///      "title": "Descriptors",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/PackageDescriptor"
    ///      }
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "stability": {
    ///      "title": "Stability",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackageGroup {
        pub descriptors: ::std::vec::Vec<PackageDescriptor>,
        pub name: ::std::string::String,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub stability: ::std::option::Option<::std::string::String>,
    }
    impl ::std::convert::From<&PackageGroup> for PackageGroup {
        fn from(value: &PackageGroup) -> Self {
            value.clone()
        }
    }
    ///`PackageGroups`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageGroups",
    ///  "examples": [
    ///    {
    ///      "items": [
    ///        {
    ///          "descriptors": [
    ///            {
    ///              "attr_path": "curl",
    ///              "install_id": "curl",
    ///              "systems": [
    ///                "x86_64-linux"
    ///              ]
    ///            },
    ///            {
    ///              "attr_path": "slack",
    ///              "install_id": "slack",
    ///              "systems": [
    ///                "x86_64-linux"
    ///              ]
    ///            },
    ///            {
    ///              "attr_path": "xorg.xeyes",
    ///              "install_id": "xeyes",
    ///              "systems": [
    ///                "x86_64-linux"
    ///              ]
    ///            }
    ///          ],
    ///          "name": "test"
    ///        }
    ///      ]
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "items"
    ///  ],
    ///  "properties": {
    ///    "items": {
    ///      "title": "Items",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/PackageGroup"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackageGroups {
        pub items: ::std::vec::Vec<PackageGroup>,
    }
    impl ::std::convert::From<&PackageGroups> for PackageGroups {
        fn from(value: &PackageGroups) -> Self {
            value.clone()
        }
    }
    ///`PackageInfoSearch`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageInfoSearch",
    ///  "examples": [
    ///    {
    ///      "attr_path": "foo.bar.curl",
    ///      "catalog": "nixpkgs",
    ///      "description": "A very nice Item",
    ///      "name": "curl",
    ///      "pkg_path": "foo.bar.curl",
    ///      "pname": "curl",
    ///      "stabilities": [
    ///        "stable",
    ///        "unstable"
    ///      ],
    ///      "system": "x86_64-linux",
    ///      "version": "1.0"
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "attr_path",
    ///    "catalog",
    ///    "description",
    ///    "name",
    ///    "pkg_path",
    ///    "pname",
    ///    "stabilities",
    ///    "system",
    ///    "version"
    ///  ],
    ///  "properties": {
    ///    "attr_path": {
    ///      "title": "Attr Path",
    ///      "type": "string"
    ///    },
    ///    "catalog": {
    ///      "title": "Catalog",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "description": {
    ///      "title": "Description",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "pkg_path": {
    ///      "title": "Pkg Path",
    ///      "type": "string"
    ///    },
    ///    "pname": {
    ///      "title": "Pname",
    ///      "type": "string"
    ///    },
    ///    "stabilities": {
    ///      "title": "Stabilities",
    ///      "type": "array",
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "system": {
    ///      "$ref": "#/components/schemas/PackageSystem"
    ///    },
    ///    "version": {
    ///      "title": "Version",
    ///      "description": "While version should always be present, (and is required in the PackageResolutionInfo model), there are cases where it has been historically optional and thus is carried forward here.  Published derivations have an Optional version and this same model is used for both published derivations and base catalog derivations.  For this reason we cannot make it required here until/if we unify those models and ensure every derivation does in fact have a version.",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackageInfoSearch {
        pub attr_path: ::std::string::String,
        pub catalog: ::std::option::Option<::std::string::String>,
        pub description: ::std::option::Option<::std::string::String>,
        pub name: ::std::string::String,
        pub pkg_path: ::std::string::String,
        pub pname: ::std::string::String,
        pub stabilities: ::std::vec::Vec<::std::string::String>,
        pub system: PackageSystem,
        ///While version should always be present, (and is required in the PackageResolutionInfo model), there are cases where it has been historically optional and thus is carried forward here.  Published derivations have an Optional version and this same model is used for both published derivations and base catalog derivations.  For this reason we cannot make it required here until/if we unify those models and ensure every derivation does in fact have a version.
        pub version: ::std::option::Option<::std::string::String>,
    }
    impl ::std::convert::From<&PackageInfoSearch> for PackageInfoSearch {
        fn from(value: &PackageInfoSearch) -> Self {
            value.clone()
        }
    }
    ///`PackageName`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "Package Name",
    ///  "examples": [
    ///    "curl"
    ///  ],
    ///  "type": "string",
    ///  "pattern": "[a-zA-Z0-9.\\-_]{3,128}"
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Serialize, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    #[serde(transparent)]
    pub struct PackageName(::std::string::String);
    impl ::std::ops::Deref for PackageName {
        type Target = ::std::string::String;
        fn deref(&self) -> &::std::string::String {
            &self.0
        }
    }
    impl ::std::convert::From<PackageName> for ::std::string::String {
        fn from(value: PackageName) -> Self {
            value.0
        }
    }
    impl ::std::convert::From<&PackageName> for PackageName {
        fn from(value: &PackageName) -> Self {
            value.clone()
        }
    }
    impl ::std::str::FromStr for PackageName {
        type Err = self::error::ConversionError;
        fn from_str(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            static PATTERN: ::std::sync::LazyLock<::regress::Regex> = ::std::sync::LazyLock::new(||
            { ::regress::Regex::new("[a-zA-Z0-9.\\-_]{3,128}").unwrap() });
            if PATTERN.find(value).is_none() {
                return Err("doesn't match pattern \"[a-zA-Z0-9.\\-_]{3,128}\"".into());
            }
            Ok(Self(value.to_string()))
        }
    }
    impl ::std::convert::TryFrom<&str> for PackageName {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<&::std::string::String> for PackageName {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<::std::string::String> for PackageName {
        type Error = self::error::ConversionError;
        fn try_from(
            value: ::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl<'de> ::serde::Deserialize<'de> for PackageName {
        fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
        where
            D: ::serde::Deserializer<'de>,
        {
            ::std::string::String::deserialize(deserializer)?
                .parse()
                .map_err(|e: self::error::ConversionError| {
                    <D::Error as ::serde::de::Error>::custom(e.to_string())
                })
        }
    }
    ///`PackageOutput`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageOutput",
    ///  "type": "object",
    ///  "required": [
    ///    "name",
    ///    "store_path"
    ///  ],
    ///  "properties": {
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "store_path": {
    ///      "title": "Store Path",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackageOutput {
        pub name: ::std::string::String,
        pub store_path: ::std::string::String,
    }
    impl ::std::convert::From<&PackageOutput> for PackageOutput {
        fn from(value: &PackageOutput) -> Self {
            value.clone()
        }
    }
    ///`PackageOutputs`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageOutputs",
    ///  "type": "array",
    ///  "items": {
    ///    "$ref": "#/components/schemas/PackageOutput"
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    #[serde(transparent)]
    pub struct PackageOutputs(pub ::std::vec::Vec<PackageOutput>);
    impl ::std::ops::Deref for PackageOutputs {
        type Target = ::std::vec::Vec<PackageOutput>;
        fn deref(&self) -> &::std::vec::Vec<PackageOutput> {
            &self.0
        }
    }
    impl ::std::convert::From<PackageOutputs> for ::std::vec::Vec<PackageOutput> {
        fn from(value: PackageOutputs) -> Self {
            value.0
        }
    }
    impl ::std::convert::From<&PackageOutputs> for PackageOutputs {
        fn from(value: &PackageOutputs) -> Self {
            value.clone()
        }
    }
    impl ::std::convert::From<::std::vec::Vec<PackageOutput>> for PackageOutputs {
        fn from(value: ::std::vec::Vec<PackageOutput>) -> Self {
            Self(value)
        }
    }
    ///`PackageResolutionInfo`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageResolutionInfo",
    ///  "type": "object",
    ///  "required": [
    ///    "attr_path",
    ///    "broken",
    ///    "derivation",
    ///    "description",
    ///    "insecure",
    ///    "license",
    ///    "locked_url",
    ///    "missing_builds",
    ///    "name",
    ///    "outputs",
    ///    "outputs_to_install",
    ///    "pkg_path",
    ///    "pname",
    ///    "rev",
    ///    "rev_count",
    ///    "rev_date",
    ///    "scrape_date",
    ///    "stabilities",
    ///    "system",
    ///    "unfree",
    ///    "version"
    ///  ],
    ///  "properties": {
    ///    "attr_path": {
    ///      "title": "Attr Path",
    ///      "type": "string"
    ///    },
    ///    "broken": {
    ///      "title": "Broken",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "cache_uri": {
    ///      "title": "Cache Uri",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "catalog": {
    ///      "title": "Catalog",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "derivation": {
    ///      "title": "Derivation",
    ///      "type": "string"
    ///    },
    ///    "description": {
    ///      "title": "Description",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "insecure": {
    ///      "title": "Insecure",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "license": {
    ///      "title": "License",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "locked_url": {
    ///      "title": "Locked Url",
    ///      "type": "string"
    ///    },
    ///    "missing_builds": {
    ///      "title": "Missing Builds",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "outputs": {
    ///      "$ref": "#/components/schemas/PackageOutputs"
    ///    },
    ///    "outputs_to_install": {
    ///      "title": "Outputs To Install",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "pkg_path": {
    ///      "title": "Pkg Path",
    ///      "type": "string"
    ///    },
    ///    "pname": {
    ///      "title": "Pname",
    ///      "type": "string"
    ///    },
    ///    "rev": {
    ///      "title": "Rev",
    ///      "type": "string"
    ///    },
    ///    "rev_count": {
    ///      "title": "Rev Count",
    ///      "type": "integer"
    ///    },
    ///    "rev_date": {
    ///      "title": "Rev Date",
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "scrape_date": {
    ///      "title": "Scrape Date",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ],
    ///      "format": "date-time"
    ///    },
    ///    "stabilities": {
    ///      "title": "Stabilities",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "system": {
    ///      "$ref": "#/components/schemas/PackageSystem"
    ///    },
    ///    "unfree": {
    ///      "title": "Unfree",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "version": {
    ///      "title": "Version",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackageResolutionInfo {
        pub attr_path: ::std::string::String,
        pub broken: ::std::option::Option<bool>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub cache_uri: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub catalog: ::std::option::Option<::std::string::String>,
        pub derivation: ::std::string::String,
        pub description: ::std::option::Option<::std::string::String>,
        pub insecure: ::std::option::Option<bool>,
        pub license: ::std::option::Option<::std::string::String>,
        pub locked_url: ::std::string::String,
        pub missing_builds: ::std::option::Option<bool>,
        pub name: ::std::string::String,
        pub outputs: PackageOutputs,
        pub outputs_to_install: ::std::option::Option<
            ::std::vec::Vec<::std::string::String>,
        >,
        pub pkg_path: ::std::string::String,
        pub pname: ::std::string::String,
        pub rev: ::std::string::String,
        pub rev_count: i64,
        pub rev_date: ::chrono::DateTime<::chrono::offset::Utc>,
        pub scrape_date: ::std::option::Option<
            ::chrono::DateTime<::chrono::offset::Utc>,
        >,
        pub stabilities: ::std::option::Option<::std::vec::Vec<::std::string::String>>,
        pub system: PackageSystem,
        pub unfree: ::std::option::Option<bool>,
        pub version: ::std::string::String,
    }
    impl ::std::convert::From<&PackageResolutionInfo> for PackageResolutionInfo {
        fn from(value: &PackageResolutionInfo) -> Self {
            value.clone()
        }
    }
    ///Request body for package SBOM endpoint (reserved for future extension).
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageSbomRequest",
    ///  "description": "Request body for package SBOM endpoint (reserved for future extension).",
    ///  "type": "object"
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    #[serde(transparent)]
    pub struct PackageSbomRequest(
        pub ::serde_json::Map<::std::string::String, ::serde_json::Value>,
    );
    impl ::std::ops::Deref for PackageSbomRequest {
        type Target = ::serde_json::Map<::std::string::String, ::serde_json::Value>;
        fn deref(
            &self,
        ) -> &::serde_json::Map<::std::string::String, ::serde_json::Value> {
            &self.0
        }
    }
    impl ::std::convert::From<PackageSbomRequest>
    for ::serde_json::Map<::std::string::String, ::serde_json::Value> {
        fn from(value: PackageSbomRequest) -> Self {
            value.0
        }
    }
    impl ::std::convert::From<&PackageSbomRequest> for PackageSbomRequest {
        fn from(value: &PackageSbomRequest) -> Self {
            value.clone()
        }
    }
    impl ::std::convert::From<
        ::serde_json::Map<::std::string::String, ::serde_json::Value>,
    > for PackageSbomRequest {
        fn from(
            value: ::serde_json::Map<::std::string::String, ::serde_json::Value>,
        ) -> Self {
            Self(value)
        }
    }
    ///`PackageSearchResult`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageSearchResult",
    ///  "examples": [
    ///    [
    ///      {
    ///        "attr_path": "foo.bar.curl",
    ///        "catalog": "nixpkgs",
    ///        "description": "A very nice Item",
    ///        "name": "curl",
    ///        "pkg_path": "foo.bar.curl",
    ///        "pname": "curl",
    ///        "stabilities": [
    ///          "stable",
    ///          "unstable"
    ///        ],
    ///        "system": "x86_64-linux",
    ///        "version": "1.0"
    ///      }
    ///    ]
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "items",
    ///    "total_count"
    ///  ],
    ///  "properties": {
    ///    "items": {
    ///      "title": "Items",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/PackageInfoSearch"
    ///      }
    ///    },
    ///    "total_count": {
    ///      "title": "Total Count",
    ///      "type": "integer"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackageSearchResult {
        pub items: ::std::vec::Vec<PackageInfoSearch>,
        pub total_count: i64,
    }
    impl ::std::convert::From<&PackageSearchResult> for PackageSearchResult {
        fn from(value: &PackageSearchResult) -> Self {
            value.clone()
        }
    }
    ///`PackageSystem`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageSystem",
    ///  "type": "string",
    ///  "enum": [
    ///    "aarch64-darwin",
    ///    "aarch64-linux",
    ///    "x86_64-darwin",
    ///    "x86_64-linux",
    ///    "invalid"
    ///  ]
    ///}
    /// ```
    /// </details>
    #[derive(
        ::serde::Deserialize,
        ::serde::Serialize,
        Clone,
        Copy,
        Debug,
        Eq,
        Hash,
        Ord,
        PartialEq,
        PartialOrd
    )]
    pub enum PackageSystem {
        #[serde(rename = "aarch64-darwin")]
        Aarch64Darwin,
        #[serde(rename = "aarch64-linux")]
        Aarch64Linux,
        #[serde(rename = "x86_64-darwin")]
        X8664Darwin,
        #[serde(rename = "x86_64-linux")]
        X8664Linux,
        #[serde(rename = "invalid")]
        Invalid,
    }
    impl ::std::convert::From<&Self> for PackageSystem {
        fn from(value: &PackageSystem) -> Self {
            value.clone()
        }
    }
    impl ::std::fmt::Display for PackageSystem {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
            match *self {
                Self::Aarch64Darwin => f.write_str("aarch64-darwin"),
                Self::Aarch64Linux => f.write_str("aarch64-linux"),
                Self::X8664Darwin => f.write_str("x86_64-darwin"),
                Self::X8664Linux => f.write_str("x86_64-linux"),
                Self::Invalid => f.write_str("invalid"),
            }
        }
    }
    impl ::std::str::FromStr for PackageSystem {
        type Err = self::error::ConversionError;
        fn from_str(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            match value {
                "aarch64-darwin" => Ok(Self::Aarch64Darwin),
                "aarch64-linux" => Ok(Self::Aarch64Linux),
                "x86_64-darwin" => Ok(Self::X8664Darwin),
                "x86_64-linux" => Ok(Self::X8664Linux),
                "invalid" => Ok(Self::Invalid),
                _ => Err("invalid value".into()),
            }
        }
    }
    impl ::std::convert::TryFrom<&str> for PackageSystem {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<&::std::string::String> for PackageSystem {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<::std::string::String> for PackageSystem {
        type Error = self::error::ConversionError;
        fn try_from(
            value: ::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    ///`PackagesResult`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackagesResult",
    ///  "type": "object",
    ///  "required": [
    ///    "items",
    ///    "total_count"
    ///  ],
    ///  "properties": {
    ///    "items": {
    ///      "title": "Items",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/PackageResolutionInfo"
    ///      }
    ///    },
    ///    "total_count": {
    ///      "title": "Total Count",
    ///      "type": "integer"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PackagesResult {
        pub items: ::std::vec::Vec<PackageResolutionInfo>,
        pub total_count: i64,
    }
    impl ::std::convert::From<&PackagesResult> for PackagesResult {
        fn from(value: &PackagesResult) -> Self {
            value.clone()
        }
    }
    ///`PageInfo`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PageInfo",
    ///  "type": "object",
    ///  "required": [
    ///    "rev",
    ///    "rev_count",
    ///    "stability_tags"
    ///  ],
    ///  "properties": {
    ///    "rev": {
    ///      "title": "Rev",
    ///      "type": "string"
    ///    },
    ///    "rev_count": {
    ///      "title": "Rev Count",
    ///      "type": "integer"
    ///    },
    ///    "stability_tags": {
    ///      "title": "Stability Tags",
    ///      "type": "array",
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PageInfo {
        pub rev: ::std::string::String,
        pub rev_count: i64,
        pub stability_tags: ::std::vec::Vec<::std::string::String>,
    }
    impl ::std::convert::From<&PageInfo> for PageInfo {
        fn from(value: &PageInfo) -> Self {
            value.clone()
        }
    }
    ///`Params`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "params",
    ///  "type": "object",
    ///  "properties": {
    ///    "resolve_package": {
    ///      "title": "Resolve Package",
    ///      "default": "cowsay",
    ///      "type": "string"
    ///    },
    ///    "resolve_systems": {
    ///      "title": "Resolve Systems",
    ///      "default": [
    ///        "x86_64-linux"
    ///      ],
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/PackageSystem"
    ///      }
    ///    },
    ///    "search_system": {
    ///      "$ref": "#/components/schemas/PackageSystem"
    ///    },
    ///    "search_term": {
    ///      "title": "Search Term",
    ///      "default": "in Go",
    ///      "type": "string"
    ///    },
    ///    "show_term": {
    ///      "title": "Show Term",
    ///      "default": "hello",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct Params {
        #[serde(default = "defaults::params_resolve_package")]
        pub resolve_package: ::std::string::String,
        #[serde(default = "defaults::params_resolve_systems")]
        pub resolve_systems: ::std::vec::Vec<PackageSystem>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub search_system: ::std::option::Option<PackageSystem>,
        #[serde(default = "defaults::params_search_term")]
        pub search_term: ::std::string::String,
        #[serde(default = "defaults::params_show_term")]
        pub show_term: ::std::string::String,
    }
    impl ::std::convert::From<&Params> for Params {
        fn from(value: &Params) -> Self {
            value.clone()
        }
    }
    impl ::std::default::Default for Params {
        fn default() -> Self {
            Self {
                resolve_package: defaults::params_resolve_package(),
                resolve_systems: defaults::params_resolve_systems(),
                search_system: Default::default(),
                search_term: defaults::params_search_term(),
                show_term: defaults::params_show_term(),
            }
        }
    }
    ///`PkgPathsResult`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PkgPathsResult",
    ///  "type": "object",
    ///  "required": [
    ///    "items",
    ///    "total_count"
    ///  ],
    ///  "properties": {
    ///    "items": {
    ///      "title": "Items",
    ///      "type": "array",
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "total_count": {
    ///      "title": "Total Count",
    ///      "type": "integer"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PkgPathsResult {
        pub items: ::std::vec::Vec<::std::string::String>,
        pub total_count: i64,
    }
    impl ::std::convert::From<&PkgPathsResult> for PkgPathsResult {
        fn from(value: &PkgPathsResult) -> Self {
            value.clone()
        }
    }
    ///`PublishInfoRequest`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PublishInfoRequest",
    ///  "type": "object"
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    #[serde(transparent)]
    pub struct PublishInfoRequest(
        pub ::serde_json::Map<::std::string::String, ::serde_json::Value>,
    );
    impl ::std::ops::Deref for PublishInfoRequest {
        type Target = ::serde_json::Map<::std::string::String, ::serde_json::Value>;
        fn deref(
            &self,
        ) -> &::serde_json::Map<::std::string::String, ::serde_json::Value> {
            &self.0
        }
    }
    impl ::std::convert::From<PublishInfoRequest>
    for ::serde_json::Map<::std::string::String, ::serde_json::Value> {
        fn from(value: PublishInfoRequest) -> Self {
            value.0
        }
    }
    impl ::std::convert::From<&PublishInfoRequest> for PublishInfoRequest {
        fn from(value: &PublishInfoRequest) -> Self {
            value.clone()
        }
    }
    impl ::std::convert::From<
        ::serde_json::Map<::std::string::String, ::serde_json::Value>,
    > for PublishInfoRequest {
        fn from(
            value: ::serde_json::Map<::std::string::String, ::serde_json::Value>,
        ) -> Self {
            Self(value)
        }
    }
    ///`PublishInfoResponseCatalog`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PublishInfoResponseCatalog",
    ///  "type": "object",
    ///  "required": [
    ///    "catalog_store_config"
    ///  ],
    ///  "properties": {
    ///    "catalog_store_config": {
    ///      "$ref": "#/components/schemas/CatalogStoreConfig"
    ///    },
    ///    "ingress_auth": {
    ///      "title": "Ingress Auth",
    ///      "type": [
    ///        "object",
    ///        "null"
    ///      ],
    ///      "additionalProperties": true
    ///    },
    ///    "ingress_uri": {
    ///      "title": "Ingress Uri",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PublishInfoResponseCatalog {
        pub catalog_store_config: crate::types::CatalogStoreConfig,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub ingress_auth: ::std::option::Option<
            ::serde_json::Map<::std::string::String, ::serde_json::Value>,
        >,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub ingress_uri: ::std::option::Option<::std::string::String>,
    }
    impl ::std::convert::From<&PublishInfoResponseCatalog>
    for PublishInfoResponseCatalog {
        fn from(value: &PublishInfoResponseCatalog) -> Self {
            value.clone()
        }
    }
    ///`PublishedCatalog`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PublishedCatalog",
    ///  "type": "object",
    ///  "required": [
    ///    "created",
    ///    "name",
    ///    "package_builds_ct",
    ///    "package_ct",
    ///    "per_package_builds_ct",
    ///    "store_type"
    ///  ],
    ///  "properties": {
    ///    "created": {
    ///      "title": "Created",
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "package_builds_ct": {
    ///      "title": "Package Builds Ct",
    ///      "type": "integer"
    ///    },
    ///    "package_ct": {
    ///      "title": "Package Ct",
    ///      "type": "integer"
    ///    },
    ///    "per_package_builds_ct": {
    ///      "title": "Per Package Builds Ct",
    ///      "type": "array",
    ///      "items": {
    ///        "type": "integer"
    ///      }
    ///    },
    ///    "store_type": {
    ///      "title": "Store Type",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PublishedCatalog {
        pub created: ::chrono::DateTime<::chrono::offset::Utc>,
        pub name: ::std::string::String,
        pub package_builds_ct: i64,
        pub package_ct: i64,
        pub per_package_builds_ct: ::std::vec::Vec<i64>,
        pub store_type: ::std::string::String,
    }
    impl ::std::convert::From<&PublishedCatalog> for PublishedCatalog {
        fn from(value: &PublishedCatalog) -> Self {
            value.clone()
        }
    }
    ///`PublishedCatalogInfo`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PublishedCatalogInfo",
    ///  "type": "object",
    ///  "required": [
    ///    "catalogs"
    ///  ],
    ///  "properties": {
    ///    "catalogs": {
    ///      "title": "Catalogs",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/PublishedCatalog"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct PublishedCatalogInfo {
        pub catalogs: ::std::vec::Vec<PublishedCatalog>,
    }
    impl ::std::convert::From<&PublishedCatalogInfo> for PublishedCatalogInfo {
        fn from(value: &PublishedCatalogInfo) -> Self {
            value.clone()
        }
    }
    ///`RawDependencyReport`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "RawDependencyReport",
    ///  "type": "object",
    ///  "required": [
    ///    "dependencies",
    ///    "storepath"
    ///  ],
    ///  "properties": {
    ///    "dependencies": {
    ///      "title": "Dependencies",
    ///      "type": "object",
    ///      "additionalProperties": {
    ///        "type": [
    ///          "array",
    ///          "null"
    ///        ],
    ///        "items": {
    ///          "type": "string"
    ///        }
    ///      }
    ///    },
    ///    "storepath": {
    ///      "title": "Storepath",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct RawDependencyReport {
        pub dependencies: ::std::collections::HashMap<
            ::std::string::String,
            ::std::option::Option<::std::vec::Vec<::std::string::String>>,
        >,
        pub storepath: ::std::string::String,
    }
    impl ::std::convert::From<&RawDependencyReport> for RawDependencyReport {
        fn from(value: &RawDependencyReport) -> Self {
            value.clone()
        }
    }
    ///`ResolutionMessageGeneral`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ResolutionMessageGeneral",
    ///  "type": "object",
    ///  "required": [
    ///    "context",
    ///    "level",
    ///    "message",
    ///    "type"
    ///  ],
    ///  "properties": {
    ///    "context": {
    ///      "title": "Context",
    ///      "type": "object",
    ///      "additionalProperties": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "level": {
    ///      "$ref": "#/components/schemas/MessageLevel"
    ///    },
    ///    "message": {
    ///      "title": "Message",
    ///      "type": "string"
    ///    },
    ///    "type": {
    ///      "$ref": "#/components/schemas/MessageType"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct ResolutionMessageGeneral {
        pub context: ::std::collections::HashMap<
            ::std::string::String,
            ::std::string::String,
        >,
        pub level: MessageLevel,
        pub message: ::std::string::String,
        #[serde(rename = "type")]
        pub type_: crate::error::MessageType,
    }
    impl ::std::convert::From<&ResolutionMessageGeneral> for ResolutionMessageGeneral {
        fn from(value: &ResolutionMessageGeneral) -> Self {
            value.clone()
        }
    }
    ///`ResolvedPackageDescriptor`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ResolvedPackageDescriptor",
    ///  "type": "object",
    ///  "required": [
    ///    "attr_path",
    ///    "broken",
    ///    "derivation",
    ///    "description",
    ///    "insecure",
    ///    "install_id",
    ///    "license",
    ///    "locked_url",
    ///    "missing_builds",
    ///    "name",
    ///    "outputs",
    ///    "outputs_to_install",
    ///    "pkg_path",
    ///    "pname",
    ///    "rev",
    ///    "rev_count",
    ///    "rev_date",
    ///    "scrape_date",
    ///    "stabilities",
    ///    "system",
    ///    "unfree",
    ///    "version"
    ///  ],
    ///  "properties": {
    ///    "attr_path": {
    ///      "title": "Attr Path",
    ///      "type": "string"
    ///    },
    ///    "broken": {
    ///      "title": "Broken",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "cache_uri": {
    ///      "title": "Cache Uri",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "catalog": {
    ///      "title": "Catalog",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "derivation": {
    ///      "title": "Derivation",
    ///      "type": "string"
    ///    },
    ///    "description": {
    ///      "title": "Description",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "insecure": {
    ///      "title": "Insecure",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "install_id": {
    ///      "title": "Install Id",
    ///      "type": "string"
    ///    },
    ///    "license": {
    ///      "title": "License",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "locked_url": {
    ///      "title": "Locked Url",
    ///      "type": "string"
    ///    },
    ///    "missing_builds": {
    ///      "title": "Missing Builds",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "outputs": {
    ///      "$ref": "#/components/schemas/PackageOutputs"
    ///    },
    ///    "outputs_to_install": {
    ///      "title": "Outputs To Install",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "pkg_path": {
    ///      "title": "Pkg Path",
    ///      "type": "string"
    ///    },
    ///    "pname": {
    ///      "title": "Pname",
    ///      "type": "string"
    ///    },
    ///    "rev": {
    ///      "title": "Rev",
    ///      "type": "string"
    ///    },
    ///    "rev_count": {
    ///      "title": "Rev Count",
    ///      "type": "integer"
    ///    },
    ///    "rev_date": {
    ///      "title": "Rev Date",
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "scrape_date": {
    ///      "title": "Scrape Date",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ],
    ///      "format": "date-time"
    ///    },
    ///    "stabilities": {
    ///      "title": "Stabilities",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "system": {
    ///      "$ref": "#/components/schemas/PackageSystem"
    ///    },
    ///    "unfree": {
    ///      "title": "Unfree",
    ///      "type": [
    ///        "boolean",
    ///        "null"
    ///      ]
    ///    },
    ///    "version": {
    ///      "title": "Version",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct ResolvedPackageDescriptor {
        pub attr_path: ::std::string::String,
        pub broken: ::std::option::Option<bool>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub cache_uri: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub catalog: ::std::option::Option<::std::string::String>,
        pub derivation: ::std::string::String,
        pub description: ::std::option::Option<::std::string::String>,
        pub insecure: ::std::option::Option<bool>,
        pub install_id: ::std::string::String,
        pub license: ::std::option::Option<::std::string::String>,
        pub locked_url: ::std::string::String,
        pub missing_builds: ::std::option::Option<bool>,
        pub name: ::std::string::String,
        pub outputs: PackageOutputs,
        pub outputs_to_install: ::std::option::Option<
            ::std::vec::Vec<::std::string::String>,
        >,
        pub pkg_path: ::std::string::String,
        pub pname: ::std::string::String,
        pub rev: ::std::string::String,
        pub rev_count: i64,
        pub rev_date: ::chrono::DateTime<::chrono::offset::Utc>,
        pub scrape_date: ::std::option::Option<
            ::chrono::DateTime<::chrono::offset::Utc>,
        >,
        pub stabilities: ::std::option::Option<::std::vec::Vec<::std::string::String>>,
        pub system: PackageSystem,
        pub unfree: ::std::option::Option<bool>,
        pub version: ::std::string::String,
    }
    impl ::std::convert::From<&ResolvedPackageDescriptor> for ResolvedPackageDescriptor {
        fn from(value: &ResolvedPackageDescriptor) -> Self {
            value.clone()
        }
    }
    ///`ResolvedPackageGroup`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ResolvedPackageGroup",
    ///  "examples": [
    ///    {
    ///      "attr_path": "curl",
    ///      "broken": false,
    ///      "catalog": "nixpkgs",
    ///      "derivation": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-curl-8.5.0.drv",
    ///      "description": "A command line tool for transferring files with URL syntax",
    ///      "insecure": false,
    ///      "license": "curl",
    ///      "locked_url": "https://github.com/flox/nixpkgs?rev=abc123def456",
    ///      "missing_builds": false,
    ///      "name": "curl-8.5.0",
    ///      "outputs": [
    ///        {
    ///          "name": "out",
    ///          "store_path": "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-curl-8.5.0"
    ///        },
    ///        {
    ///          "name": "man",
    ///          "store_path": "/nix/store/cccccccccccccccccccccccccccccccc-curl-8.5.0-man"
    ///        }
    ///      ],
    ///      "outputs_to_install": [
    ///        "out",
    ///        "man"
    ///      ],
    ///      "pkg_path": "curl",
    ///      "pname": "curl",
    ///      "rev": "abc123def456",
    ///      "rev_count": 12345,
    ///      "rev_date": "2024-01-15T00:00:00Z",
    ///      "stabilities": [
    ///        "stable"
    ///      ],
    ///      "system": "x86_64-linux",
    ///      "unfree": false,
    ///      "version": "8.5.0"
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "messages",
    ///    "name"
    ///  ],
    ///  "properties": {
    ///    "candidate_pages": {
    ///      "title": "Candidate Pages",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "$ref": "#/components/schemas/CatalogPage"
    ///      }
    ///    },
    ///    "messages": {
    ///      "title": "Messages",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/ResolutionMessageGeneral"
    ///      }
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "page": {
    ///      "$ref": "#/components/schemas/CatalogPage"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct ResolvedPackageGroup {
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub candidate_pages: ::std::option::Option<::std::vec::Vec<CatalogPage>>,
        pub messages: ::std::vec::Vec<ResolutionMessageGeneral>,
        pub name: ::std::string::String,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub page: ::std::option::Option<CatalogPage>,
    }
    impl ::std::convert::From<&ResolvedPackageGroup> for ResolvedPackageGroup {
        fn from(value: &ResolvedPackageGroup) -> Self {
            value.clone()
        }
    }
    ///`ResolvedPackageGroups`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ResolvedPackageGroups",
    ///  "type": "object",
    ///  "required": [
    ///    "items"
    ///  ],
    ///  "properties": {
    ///    "items": {
    ///      "title": "Items",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/ResolvedPackageGroup"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct ResolvedPackageGroups {
        pub items: ::std::vec::Vec<ResolvedPackageGroup>,
    }
    impl ::std::convert::From<&ResolvedPackageGroups> for ResolvedPackageGroups {
        fn from(value: &ResolvedPackageGroups) -> Self {
            value.clone()
        }
    }
    ///Supported SBOM format types.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "SbomFormat",
    ///  "description": "Supported SBOM format types.",
    ///  "type": "string",
    ///  "enum": [
    ///    "spdx-2.3-json"
    ///  ]
    ///}
    /// ```
    /// </details>
    #[derive(
        ::serde::Deserialize,
        ::serde::Serialize,
        Clone,
        Copy,
        Debug,
        Eq,
        Hash,
        Ord,
        PartialEq,
        PartialOrd
    )]
    pub enum SbomFormat {
        #[serde(rename = "spdx-2.3-json")]
        Spdx23Json,
    }
    impl ::std::convert::From<&Self> for SbomFormat {
        fn from(value: &SbomFormat) -> Self {
            value.clone()
        }
    }
    impl ::std::fmt::Display for SbomFormat {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
            match *self {
                Self::Spdx23Json => f.write_str("spdx-2.3-json"),
            }
        }
    }
    impl ::std::str::FromStr for SbomFormat {
        type Err = self::error::ConversionError;
        fn from_str(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            match value {
                "spdx-2.3-json" => Ok(Self::Spdx23Json),
                _ => Err("invalid value".into()),
            }
        }
    }
    impl ::std::convert::TryFrom<&str> for SbomFormat {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<&::std::string::String> for SbomFormat {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<::std::string::String> for SbomFormat {
        type Error = self::error::ConversionError;
        fn try_from(
            value: ::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    ///`SearchTerm`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "Search Term",
    ///  "type": "string",
    ///  "pattern": "[a-zA-Z0-9\\-\\.\\\\@%_,]{2,200}"
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Serialize, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    #[serde(transparent)]
    pub struct SearchTerm(::std::string::String);
    impl ::std::ops::Deref for SearchTerm {
        type Target = ::std::string::String;
        fn deref(&self) -> &::std::string::String {
            &self.0
        }
    }
    impl ::std::convert::From<SearchTerm> for ::std::string::String {
        fn from(value: SearchTerm) -> Self {
            value.0
        }
    }
    impl ::std::convert::From<&SearchTerm> for SearchTerm {
        fn from(value: &SearchTerm) -> Self {
            value.clone()
        }
    }
    impl ::std::str::FromStr for SearchTerm {
        type Err = self::error::ConversionError;
        fn from_str(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            static PATTERN: ::std::sync::LazyLock<::regress::Regex> = ::std::sync::LazyLock::new(||
            { ::regress::Regex::new("[a-zA-Z0-9\\-\\.\\\\@%_,]{2,200}").unwrap() });
            if PATTERN.find(value).is_none() {
                return Err(
                    "doesn't match pattern \"[a-zA-Z0-9\\-\\.\\\\@%_,]{2,200}\"".into(),
                );
            }
            Ok(Self(value.to_string()))
        }
    }
    impl ::std::convert::TryFrom<&str> for SearchTerm {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<&::std::string::String> for SearchTerm {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<::std::string::String> for SearchTerm {
        type Error = self::error::ConversionError;
        fn try_from(
            value: ::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl<'de> ::serde::Deserialize<'de> for SearchTerm {
        fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
        where
            D: ::serde::Deserializer<'de>,
        {
            ::std::string::String::deserialize(deserializer)?
                .parse()
                .map_err(|e: self::error::ConversionError| {
                    <D::Error as ::serde::de::Error>::custom(e.to_string())
                })
        }
    }
    ///`ServiceStatus`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ServiceStatus",
    ///  "type": "object",
    ///  "required": [
    ///    "service_version",
    ///    "start_tm",
    ///    "uptime_pretty",
    ///    "uptime_seconds"
    ///  ],
    ///  "properties": {
    ///    "service_version": {
    ///      "title": "Service Version",
    ///      "type": "string"
    ///    },
    ///    "start_tm": {
    ///      "title": "Start Tm",
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "uptime_pretty": {
    ///      "title": "Uptime Pretty",
    ///      "readOnly": true,
    ///      "type": "string"
    ///    },
    ///    "uptime_seconds": {
    ///      "title": "Uptime Seconds",
    ///      "readOnly": true,
    ///      "type": "number"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct ServiceStatus {
        pub service_version: ::std::string::String,
        pub start_tm: ::chrono::DateTime<::chrono::offset::Utc>,
        pub uptime_pretty: ::std::string::String,
        pub uptime_seconds: f64,
    }
    impl ::std::convert::From<&ServiceStatus> for ServiceStatus {
        fn from(value: &ServiceStatus) -> Self {
            value.clone()
        }
    }
    ///`StabilityInfo`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "StabilityInfo",
    ///  "type": "object",
    ///  "required": [
    ///    "name",
    ///    "ref"
    ///  ],
    ///  "properties": {
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "ref": {
    ///      "title": "Ref",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct StabilityInfo {
        pub name: ::std::string::String,
        #[serde(rename = "ref")]
        pub ref_: ::std::string::String,
    }
    impl ::std::convert::From<&StabilityInfo> for StabilityInfo {
        fn from(value: &StabilityInfo) -> Self {
            value.clone()
        }
    }
    ///`StoreInfo`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "StoreInfo",
    ///  "type": "object",
    ///  "properties": {
    ///    "auth": {
    ///      "title": "Auth",
    ///      "type": [
    ///        "object",
    ///        "null"
    ///      ],
    ///      "additionalProperties": true
    ///    },
    ///    "catalog": {
    ///      "title": "Catalog",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "package": {
    ///      "title": "Package",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "public_keys": {
    ///      "title": "Public Keys",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "url": {
    ///      "title": "Url",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct StoreInfo {
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub auth: ::std::option::Option<
            ::serde_json::Map<::std::string::String, ::serde_json::Value>,
        >,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub catalog: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub package: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub public_keys: ::std::option::Option<::std::vec::Vec<::std::string::String>>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub url: ::std::option::Option<::std::string::String>,
    }
    impl ::std::convert::From<&StoreInfo> for StoreInfo {
        fn from(value: &StoreInfo) -> Self {
            value.clone()
        }
    }
    impl ::std::default::Default for StoreInfo {
        fn default() -> Self {
            Self {
                auth: Default::default(),
                catalog: Default::default(),
                package: Default::default(),
                public_keys: Default::default(),
                url: Default::default(),
            }
        }
    }
    ///`StoreInfoRequest`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "StoreInfoRequest",
    ///  "type": "object",
    ///  "required": [
    ///    "outpaths"
    ///  ],
    ///  "properties": {
    ///    "outpaths": {
    ///      "title": "Outpaths",
    ///      "type": "array",
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct StoreInfoRequest {
        pub outpaths: ::std::vec::Vec<::std::string::String>,
    }
    impl ::std::convert::From<&StoreInfoRequest> for StoreInfoRequest {
        fn from(value: &StoreInfoRequest) -> Self {
            value.clone()
        }
    }
    ///`StoreInfoResponse`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "StoreInfoResponse",
    ///  "type": "object",
    ///  "required": [
    ///    "items"
    ///  ],
    ///  "properties": {
    ///    "items": {
    ///      "title": "Items",
    ///      "type": "object",
    ///      "additionalProperties": {
    ///        "type": "array",
    ///        "items": {
    ///          "$ref": "#/components/schemas/StoreInfo"
    ///        }
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct StoreInfoResponse {
        pub items: ::std::collections::HashMap<
            ::std::string::String,
            ::std::vec::Vec<StoreInfo>,
        >,
    }
    impl ::std::convert::From<&StoreInfoResponse> for StoreInfoResponse {
        fn from(value: &StoreInfoResponse) -> Self {
            value.clone()
        }
    }
    ///`StorepathStatus`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "StorepathStatus",
    ///  "type": "object",
    ///  "required": [
    ///    "catalog",
    ///    "narinfo_known",
    ///    "package"
    ///  ],
    ///  "properties": {
    ///    "catalog": {
    ///      "title": "Catalog",
    ///      "type": "string"
    ///    },
    ///    "narinfo_known": {
    ///      "title": "Narinfo Known",
    ///      "type": "boolean"
    ///    },
    ///    "package": {
    ///      "title": "Package",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct StorepathStatus {
        pub catalog: ::std::string::String,
        pub narinfo_known: bool,
        pub package: ::std::string::String,
    }
    impl ::std::convert::From<&StorepathStatus> for StorepathStatus {
        fn from(value: &StorepathStatus) -> Self {
            value.clone()
        }
    }
    ///`StorepathStatusResponse`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "StorepathStatusResponse",
    ///  "type": "object",
    ///  "required": [
    ///    "items"
    ///  ],
    ///  "properties": {
    ///    "items": {
    ///      "title": "Items",
    ///      "type": "object",
    ///      "additionalProperties": {
    ///        "type": "array",
    ///        "items": {
    ///          "$ref": "#/components/schemas/StorepathStatus"
    ///        }
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct StorepathStatusResponse {
        pub items: ::std::collections::HashMap<
            ::std::string::String,
            ::std::vec::Vec<StorepathStatus>,
        >,
    }
    impl ::std::convert::From<&StorepathStatusResponse> for StorepathStatusResponse {
        fn from(value: &StorepathStatusResponse) -> Self {
            value.clone()
        }
    }
    ///`UserCatalog`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "UserCatalog",
    ///  "type": "object",
    ///  "required": [
    ///    "created_at",
    ///    "id",
    ///    "name"
    ///  ],
    ///  "properties": {
    ///    "created_at": {
    ///      "title": "Created At",
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "id": {
    ///      "title": "Id",
    ///      "type": "integer"
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "owner_handle": {
    ///      "title": "Owner Handle",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct UserCatalog {
        pub created_at: ::chrono::DateTime<::chrono::offset::Utc>,
        pub id: i64,
        pub name: ::std::string::String,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub owner_handle: ::std::option::Option<::std::string::String>,
    }
    impl ::std::convert::From<&UserCatalog> for UserCatalog {
        fn from(value: &UserCatalog) -> Self {
            value.clone()
        }
    }
    ///`UserPackage`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "UserPackage",
    ///  "type": "object",
    ///  "required": [
    ///    "catalog",
    ///    "name"
    ///  ],
    ///  "properties": {
    ///    "catalog": {
    ///      "title": "Catalog",
    ///      "type": "string"
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "original_url": {
    ///      "title": "Original Url",
    ///      "deprecated": true,
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct UserPackage {
        pub catalog: ::std::string::String,
        pub name: ::std::string::String,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub original_url: ::std::option::Option<::std::string::String>,
    }
    impl ::std::convert::From<&UserPackage> for UserPackage {
        fn from(value: &UserPackage) -> Self {
            value.clone()
        }
    }
    ///`UserPackageCreate`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "UserPackageCreate",
    ///  "type": "object",
    ///  "properties": {
    ///    "original_url": {
    ///      "title": "Original Url",
    ///      "deprecated": true,
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct UserPackageCreate {
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub original_url: ::std::option::Option<::std::string::String>,
    }
    impl ::std::convert::From<&UserPackageCreate> for UserPackageCreate {
        fn from(value: &UserPackageCreate) -> Self {
            value.clone()
        }
    }
    impl ::std::default::Default for UserPackageCreate {
        fn default() -> Self {
            Self {
                original_url: Default::default(),
            }
        }
    }
    ///`UserPackageList`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "UserPackageList",
    ///  "type": "object",
    ///  "required": [
    ///    "items"
    ///  ],
    ///  "properties": {
    ///    "items": {
    ///      "title": "Items",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/UserPackage"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct UserPackageList {
        pub items: ::std::vec::Vec<UserPackage>,
    }
    impl ::std::convert::From<&UserPackageList> for UserPackageList {
        fn from(value: &UserPackageList) -> Self {
            value.clone()
        }
    }
    /// Generation of default values for serde.
    pub mod defaults {
        pub(super) fn catalog_share_info_allow_read_users() -> ::std::option::Option<
            ::std::vec::Vec<::std::string::String>,
        > {
            ::std::option::Option::Some(vec![])
        }
        pub(super) fn catalog_store_config_meta_only_store_type() -> ::std::string::String {
            "meta-only".to_string()
        }
        pub(super) fn catalog_store_config_nix_copy_store_type() -> ::std::string::String {
            "nix-copy".to_string()
        }
        pub(super) fn catalog_store_config_null_store_type() -> ::std::string::String {
            "null".to_string()
        }
        pub(super) fn catalog_store_config_publisher_store_type() -> ::std::string::String {
            "publisher".to_string()
        }
        pub(super) fn package_descriptor_allow_broken() -> ::std::option::Option<bool> {
            ::std::option::Option::Some(false)
        }
        pub(super) fn package_descriptor_allow_insecure() -> ::std::option::Option<
            bool,
        > {
            ::std::option::Option::Some(false)
        }
        pub(super) fn package_descriptor_allow_pre_releases() -> ::std::option::Option<
            bool,
        > {
            ::std::option::Option::Some(false)
        }
        pub(super) fn package_descriptor_allow_unfree() -> ::std::option::Option<bool> {
            ::std::option::Option::Some(true)
        }
        pub(super) fn params_resolve_package() -> ::std::string::String {
            "cowsay".to_string()
        }
        pub(super) fn params_resolve_systems() -> ::std::vec::Vec<super::PackageSystem> {
            vec![super::PackageSystem::X8664Linux]
        }
        pub(super) fn params_search_term() -> ::std::string::String {
            "in Go".to_string()
        }
        pub(super) fn params_show_term() -> ::std::string::String {
            "hello".to_string()
        }
    }
}
#[derive(Clone, Debug)]
/**Client for Floxhub Catalog Server


# Floxhub Catalog Service API

![packages](https://api.preview.flox.dev/api/v1/catalog/status/badges/packages.svg)


Version: unknown*/
pub struct Client {
    pub(crate) baseurl: String,
    pub(crate) client: reqwest::Client,
}
impl Client {
    /// Create a new client.
    ///
    /// `baseurl` is the base URL provided to the internal
    /// `reqwest::Client`, and should include a scheme and hostname,
    /// as well as port and a path stem if applicable.
    pub fn new(baseurl: &str) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let client = {
            let dur = ::std::time::Duration::from_secs(15u64);
            reqwest::ClientBuilder::new().connect_timeout(dur).timeout(dur)
        };
        #[cfg(target_arch = "wasm32")]
        let client = reqwest::ClientBuilder::new();
        Self::new_with_client(baseurl, client.build().unwrap())
    }
    /// Construct a new client with an existing `reqwest::Client`,
    /// allowing more control over its configuration.
    ///
    /// `baseurl` is the base URL provided to the internal
    /// `reqwest::Client`, and should include a scheme and hostname,
    /// as well as port and a path stem if applicable.
    pub fn new_with_client(baseurl: &str, client: reqwest::Client) -> Self {
        Self {
            baseurl: baseurl.to_string(),
            client,
        }
    }
}
impl ClientInfo<()> for Client {
    fn api_version() -> &'static str {
        "unknown"
    }
    fn baseurl(&self) -> &str {
        self.baseurl.as_str()
    }
    fn client(&self) -> &reqwest::Client {
        &self.client
    }
    fn inner(&self) -> &() {
        &()
    }
}
impl ClientHooks<()> for &Client {}
#[allow(clippy::all)]
impl Client {
    /**Create a new user catalog

Create a new user catalog

Required Query Parameters:
- **name**: The name of the new catalog


Returns:
- **UserCatalog**: The new user catalog

Sends a `POST` request to `/api/v1/catalog/catalogs/`

*/
    pub async fn create_catalog_api_v1_catalog_catalogs_post<'a>(
        &'a self,
        name: &'a types::CatalogName,
    ) -> Result<ResponseValue<types::UserCatalog>, Error<types::ErrorResponse>> {
        let url = format!("{}/api/v1/catalog/catalogs/", self.baseurl,);
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .query(&progenitor_client::QueryParam::new("name", &name))
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "create_catalog_api_v1_catalog_catalogs_post",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            201u16 => ResponseValue::from_response(response).await,
            409u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get user catalog metadata

Get user catalog metadata.

Path Parameters:
- **catalog_name**: The name of the catalog

Returns:
- **UserCatalog**: The user catalog metadata including id, name, created_at,
  and owner_handle.

Sends a `GET` request to `/api/v1/catalog/catalogs/{catalog_name}`

*/
    pub async fn get_catalog_api_v1_catalog_catalogs_catalog_name_get<'a>(
        &'a self,
        catalog_name: &'a types::CatalogName,
    ) -> Result<ResponseValue<types::UserCatalog>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}", self.baseurl, encode_path(& catalog_name
            .to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "get_catalog_api_v1_catalog_catalogs_catalog_name_get",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Delete a user catalog

Delete a user catalog.

Path Parameters:
- **catalog_name**: The name of catalog to delete

Returns:
- **None**

Note: This endpoint is not yet implemented and returns 501.

Sends a `DELETE` request to `/api/v1/catalog/catalogs/{catalog_name}`

*/
    pub async fn delete_catalog_api_v1_catalog_catalogs_catalog_name_delete<'a>(
        &'a self,
        catalog_name: &'a types::CatalogName,
    ) -> Result<ResponseValue<::serde_json::Value>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}", self.baseurl, encode_path(& catalog_name
            .to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .delete(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "delete_catalog_api_v1_catalog_catalogs_catalog_name_delete",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            501u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**List packages available in a catalog

Lists available packages in a catalog

Path Parameters:
- **catalog_name**: The name of the catalog

Returns:
- **UserPackageList**

Sends a `GET` request to `/api/v1/catalog/catalogs/{catalog_name}/packages`

*/
    pub async fn get_catalog_packages_api_v1_catalog_catalogs_catalog_name_packages_get<
        'a,
    >(
        &'a self,
        catalog_name: &'a types::CatalogName,
    ) -> Result<ResponseValue<types::UserPackageList>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}/packages", self.baseurl, encode_path(&
            catalog_name.to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "get_catalog_packages_api_v1_catalog_catalogs_catalog_name_packages_get",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Create a new package in a user catalog

Creates a catalog package

Path Parameters:
- **catalog_name**: The name of catalog to place the package into

Required Query Parameters:
- **name**: The name of package (attr_path) to create

Returns:
- **UserPackage**

Sends a `POST` request to `/api/v1/catalog/catalogs/{catalog_name}/packages`

*/
    pub async fn create_catalog_package_api_v1_catalog_catalogs_catalog_name_packages_post<
        'a,
    >(
        &'a self,
        catalog_name: &'a types::CatalogName,
        name: &'a types::PackageName,
        body: &'a types::UserPackageCreate,
    ) -> Result<ResponseValue<types::UserPackage>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}/packages", self.baseurl, encode_path(&
            catalog_name.to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .query(&progenitor_client::QueryParam::new("name", &name))
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "create_catalog_package_api_v1_catalog_catalogs_catalog_name_packages_post",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            201u16 => ResponseValue::from_response(response).await,
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            409u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get package info

Get package info

Path Parameters:
- **catalog_name**: The name of the catalog
- **package_name**: The name of the package

Returns:
- **UserPackage**

Note: Authentication is optional. Unauthenticated users can access
packages in public catalogs (flox, base). Private catalogs require
authentication.

Sends a `GET` request to `/api/v1/catalog/catalogs/{catalog_name}/packages/{package_name}`

*/
    pub async fn get_catalog_package_api_v1_catalog_catalogs_catalog_name_packages_package_name_get<
        'a,
    >(
        &'a self,
        catalog_name: &'a types::CatalogName,
        package_name: &'a types::PackageName,
    ) -> Result<ResponseValue<types::UserPackage>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}/packages/{}", self.baseurl, encode_path(&
            catalog_name.to_string()), encode_path(& package_name.to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "get_catalog_package_api_v1_catalog_catalogs_catalog_name_packages_package_name_get",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get a list of builds for a given package

Get the list of builds for a given package

Path Parameters:
- **catalog_name**: The name of the catalog
- **package_name**: The name of the package

Returns:
- **PackageBuildList**

Sends a `GET` request to `/api/v1/catalog/catalogs/{catalog_name}/packages/{package_name}/builds`

*/
    pub async fn get_package_builds_api_v1_catalog_catalogs_catalog_name_packages_package_name_builds_get<
        'a,
    >(
        &'a self,
        catalog_name: &'a types::CatalogName,
        package_name: &'a types::PackageName,
    ) -> Result<ResponseValue<types::PackageBuildList>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}/packages/{}/builds", self.baseurl,
            encode_path(& catalog_name.to_string()), encode_path(& package_name
            .to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "get_package_builds_api_v1_catalog_catalogs_catalog_name_packages_package_name_builds_get",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Update a build of a particular package

Create or update a build of a package

Path Parameters:
- **catalog_name**: The name of the catalog
- **package_name**: The name of the package
Body Content:
- **PackageBuild**: The build info to submit

Returns:
- **PackageBuildResponse**

Sends a `PUT` request to `/api/v1/catalog/catalogs/{catalog_name}/packages/{package_name}/builds`

*/
    pub async fn create_package_build_api_v1_catalog_catalogs_catalog_name_packages_package_name_builds_put<
        'a,
    >(
        &'a self,
        catalog_name: &'a types::CatalogName,
        package_name: &'a types::PackageName,
        body: &'a types::PackageBuildWithNarInfo,
    ) -> Result<
        ResponseValue<types::PackageBuildResponse>,
        Error<types::ErrorResponse>,
    > {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}/packages/{}/builds", self.baseurl,
            encode_path(& catalog_name.to_string()), encode_path(& package_name
            .to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .put(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "create_package_build_api_v1_catalog_catalogs_catalog_name_packages_package_name_builds_put",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            201u16 => ResponseValue::from_response(response).await,
            400u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Submit a build of a particular package

Create or update a build of a package

Path Parameters:
- **catalog_name**: The name of the catalog
- **package_name**: The name of the package
Body Content:
- **PackageBuild**: The build info to submit

Returns:
- **PackageBuildResponse**

Sends a `POST` request to `/api/v1/catalog/catalogs/{catalog_name}/packages/{package_name}/builds`

*/
    pub async fn create_package_build_api_v1_catalog_catalogs_catalog_name_packages_package_name_builds_post<
        'a,
    >(
        &'a self,
        catalog_name: &'a types::CatalogName,
        package_name: &'a types::PackageName,
        body: &'a types::PackageBuildWithNarInfo,
    ) -> Result<
        ResponseValue<types::PackageBuildResponse>,
        Error<types::ErrorResponse>,
    > {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}/packages/{}/builds", self.baseurl,
            encode_path(& catalog_name.to_string()), encode_path(& package_name
            .to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "create_package_build_api_v1_catalog_catalogs_catalog_name_packages_package_name_builds_post",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            201u16 => ResponseValue::from_response(response).await,
            400u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Request access and info to publish a package

Request access and informatin to publish a package to this catalog.
Path Parameters:
- **catalog_name**: The name of the catalog
- **package_name**: The name of the package
Body Content:
- **PublishInfoRequest**: The information needed to publish to the catalog
Returns:
- **PublishRequestResponse**

Sends a `POST` request to `/api/v1/catalog/catalogs/{catalog_name}/packages/{package_name}/publish/info`

*/
    pub async fn publish_request_api_v1_catalog_catalogs_catalog_name_packages_package_name_publish_info_post<
        'a,
    >(
        &'a self,
        catalog_name: &'a types::CatalogName,
        package_name: &'a types::PackageName,
        body: &'a types::PublishInfoRequest,
    ) -> Result<
        ResponseValue<types::PublishInfoResponseCatalog>,
        Error<types::ErrorResponse>,
    > {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}/packages/{}/publish/info", self.baseurl,
            encode_path(& catalog_name.to_string()), encode_path(& package_name
            .to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "publish_request_api_v1_catalog_catalogs_catalog_name_packages_package_name_publish_info_post",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            400u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get usernames that have read access to a catalog

Get the list of usernames with read access to a catalog

Path Parameters:
- **catalog_name**: The name of the catalog

Returns:
- **CatalogShareInfo**: The users with read access to the catalog

Sends a `GET` request to `/api/v1/catalog/catalogs/{catalog_name}/sharing`

*/
    pub async fn get_catalog_sharing_api_v1_catalog_catalogs_catalog_name_sharing_get<
        'a,
    >(
        &'a self,
        catalog_name: &'a types::CatalogName,
    ) -> Result<ResponseValue<types::CatalogShareInfo>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}/sharing", self.baseurl, encode_path(&
            catalog_name.to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "get_catalog_sharing_api_v1_catalog_catalogs_catalog_name_sharing_get",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Add usernames to the read access list for a catalog

Add usernames to the read access list for a catalog

Path Parameters:
- **catalog_name**: The name of the catalog

Body Content:
- **CatalogShareInfo**: The users to add to the read access list

Returns:
- **CatalogShareInfo**: The users with read access to the catalog

Sends a `POST` request to `/api/v1/catalog/catalogs/{catalog_name}/sharing/add-read-users`

*/
    pub async fn add_catalog_sharing_api_v1_catalog_catalogs_catalog_name_sharing_add_read_users_post<
        'a,
    >(
        &'a self,
        catalog_name: &'a types::CatalogName,
        body: &'a types::CatalogShareInfo,
    ) -> Result<ResponseValue<types::CatalogShareInfo>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}/sharing/add-read-users", self.baseurl,
            encode_path(& catalog_name.to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "add_catalog_sharing_api_v1_catalog_catalogs_catalog_name_sharing_add_read_users_post",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Remove usernames from the read access list for a catalog

Remove usernames from the read access list for a catalog

Path Parameters:
- **catalog_name**: The name of the catalog

Body Content:
- **CatalogShareInfo**: The users to remove from the read access list

Returns:
- **CatalogShareInfo**: The users with read access to the catalog

Sends a `POST` request to `/api/v1/catalog/catalogs/{catalog_name}/sharing/remove-read-users`

*/
    pub async fn remove_catalog_sharing_api_v1_catalog_catalogs_catalog_name_sharing_remove_read_users_post<
        'a,
    >(
        &'a self,
        catalog_name: &'a types::CatalogName,
        body: &'a types::CatalogShareInfo,
    ) -> Result<ResponseValue<types::CatalogShareInfo>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}/sharing/remove-read-users", self.baseurl,
            encode_path(& catalog_name.to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "remove_catalog_sharing_api_v1_catalog_catalogs_catalog_name_sharing_remove_read_users_post",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get store config

Get store configuration for a catalog.

Path Parameters:
- **catalog_name**: The name of the catalog

Returns:
- **CatalogStoreConfig**: The store configuration (null, meta-only,
  nix-copy, or publisher type) with associated URIs if applicable.

Sends a `GET` request to `/api/v1/catalog/catalogs/{catalog_name}/store/config`

*/
    pub async fn get_catalog_store_config_api_v1_catalog_catalogs_catalog_name_store_config_get<
        'a,
    >(
        &'a self,
        catalog_name: &'a types::CatalogName,
    ) -> Result<
        ResponseValue<crate::types::CatalogStoreConfig>,
        Error<types::ErrorResponse>,
    > {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}/store/config", self.baseurl, encode_path(&
            catalog_name.to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "get_catalog_store_config_api_v1_catalog_catalogs_catalog_name_store_config_get",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Set store config

Update store configuration for a catalog.

Path Parameters:
- **catalog_name**: The name of the catalog

Body Parameters:
- **CatalogStoreConfig**: The new store configuration to set

Returns:
- **CatalogStoreConfig**: The updated store configuration.

Sends a `PUT` request to `/api/v1/catalog/catalogs/{catalog_name}/store/config`

*/
    pub async fn set_catalog_store_config_api_v1_catalog_catalogs_catalog_name_store_config_put<
        'a,
    >(
        &'a self,
        catalog_name: &'a types::CatalogName,
        body: &'a crate::types::CatalogStoreConfig,
    ) -> Result<
        ResponseValue<crate::types::CatalogStoreConfig>,
        Error<types::ErrorResponse>,
    > {
        let url = format!(
            "{}/api/v1/catalog/catalogs/{}/store/config", self.baseurl, encode_path(&
            catalog_name.to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .put(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "set_catalog_store_config_api_v1_catalog_catalogs_catalog_name_store_config_put",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get base catalog information

Sends a `GET` request to `/api/v1/catalog/info/base-catalog`

*/
    pub async fn get_base_catalog_api_v1_catalog_info_base_catalog_get<'a>(
        &'a self,
    ) -> Result<ResponseValue<types::BaseCatalogInfo>, Error<types::ErrorResponse>> {
        let url = format!("{}/api/v1/catalog/info/base-catalog", self.baseurl,);
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "get_base_catalog_api_v1_catalog_info_base_catalog_get",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get a raw dependency report for a storepath (excluding /nix/store/ prefix)

Get a raw dependency report for a store path.

Path Parameters:
- **storepath**: The store path to analyze (excluding /nix/store/ prefix)

Returns:
- **RawDependencyReport**: Dependency graph with all transitive dependencies,
  including store paths, derivation info, and dependency relationships.

Note: Requires authentication and flox organization membership.

Sends a `GET` request to `/api/v1/catalog/info/dependencies/{storepath}/raw`

*/
    pub async fn raw_dependency_report_api_v1_catalog_info_dependencies_storepath_raw_get<
        'a,
    >(
        &'a self,
        storepath: &'a str,
    ) -> Result<ResponseValue<types::RawDependencyReport>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/info/dependencies/{}/raw", self.baseurl, encode_path(&
            storepath.to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "raw_dependency_report_api_v1_catalog_info_dependencies_storepath_raw_get",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get an SPDX report for a storepath (excluding /nix/store/ prefix)

Get an SPDX 2.3 format dependency report for a store path.

Path Parameters:
- **storepath**: The store path to analyze (excluding /nix/store/ prefix)

Returns:
- **dict**: SPDX 2.3 JSON document containing package information and
  dependency relationships in standard SBOM format.

Note: Requires authentication and flox organization membership.

Sends a `GET` request to `/api/v1/catalog/info/dependencies/{storepath}/spdx`

*/
    pub async fn spdx_report_api_v1_catalog_info_dependencies_storepath_spdx_get<'a>(
        &'a self,
        storepath: &'a str,
    ) -> Result<
        ResponseValue<::serde_json::Map<::std::string::String, ::serde_json::Value>>,
        Error<types::ErrorResponse>,
    > {
        let url = format!(
            "{}/api/v1/catalog/info/dependencies/{}/spdx", self.baseurl, encode_path(&
            storepath.to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "spdx_report_api_v1_catalog_info_dependencies_storepath_spdx_get",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Shows available packages of a specific package

Returns a list of versions for a given attr_path

Required Query Parameters:
- **attr_path**: The attr_path, must be valid.

Optional Query Parameters:
- **page**: Optional page number for pagination (def = 0)
- **pageSize**: Optional page size for pagination (def = 10)

Returns:
- **PackagesResult**: A list of PackageResolutionInfo and the total result count

Sends a `GET` request to `/api/v1/catalog/packages/{attr_path}`

*/
    pub async fn packages_api_v1_catalog_packages_attr_path_get<'a>(
        &'a self,
        attr_path: &'a str,
        page: Option<i64>,
        page_size: Option<i64>,
    ) -> Result<ResponseValue<types::PackagesResult>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/packages/{}", self.baseurl, encode_path(& attr_path
            .to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .query(&progenitor_client::QueryParam::new("page", &page))
            .query(&progenitor_client::QueryParam::new("pageSize", &page_size))
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "packages_api_v1_catalog_packages_attr_path_get",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Resolve a list of Package Groups

Resolves a list of package groups, each being a list of package descriptors.

Required Body:
- **groups**: An object with an `items` array of PackageGroups to resolve.

Optional Query Parameters:
- **candidate_pages**: Number of additional candidate pages to return
  (default: 0)

Returns:
- **ResolvedPackageGroups**: An object with an `items` array of
  `ResolvedPackageGroup` items.

Resolution Rules:
- Each `PackageGroup` is resolved independently.
- Each page that has packages meeting all descriptors in the group is
  returned.
- The latest complete page includes full package details.
- Additional candidate pages are returned without full details.

PackageDescriptor Fields:
- **install_id**: [required] Reference identifier for the package in the
  manifest. Used for error messages and result correlation.
- **attr_path**: [required] The nix attribute path to match exactly.
- **systems**: [required] List of systems to resolve for (e.g.,
  x86_64-linux).
- **version**: [optional] Version constraint. Can be a literal version or
  semver constraint. Packages whose version cannot be parsed as semver are
  excluded when using semver constraints.
- **derivation**: [optional] Specific derivation path to match.
- **allow_pre_releases**: [optional] Include pre-release versions when using
  semver constraints (default: False).
- **allow_broken**: [optional] Include packages marked as broken
  (default: False).
- **allow_unfree**: [optional] Include packages with unfree licenses
  (default: True).
- **allow_insecure**: [optional] Include packages marked as insecure
  (default: False).
- **allowed_licenses**: [optional] List of acceptable license identifiers.
- **allow_missing_builds**: [optional] Include packages without confirmed
  build artifacts (default: False). If resolution fails with this
  constraint, it may be relaxed with a warning message.

Sends a `POST` request to `/api/v1/catalog/resolve`

*/
    pub async fn resolve_api_v1_catalog_resolve_post<'a>(
        &'a self,
        candidate_pages: Option<i64>,
        body: &'a types::PackageGroups,
    ) -> Result<
        ResponseValue<types::ResolvedPackageGroups>,
        Error<types::ErrorResponse>,
    > {
        let url = format!("{}/api/v1/catalog/resolve", self.baseurl,);
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .query(
                &progenitor_client::QueryParam::new("candidate_pages", &candidate_pages),
            )
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "resolve_api_v1_catalog_resolve_post",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get SBOM for an environment

Get SBOM (Software Bill of Materials) for an environment.

Args:
    body: Request body containing lockfile and environment metadata
    format: SBOM format (defaults to SbomFormat.SPDX_2_3_JSON)
    auth_result: Authentication payload
    cache: Request-scoped dependency cache (injected)

Returns:
    SBOM document in the requested format

Raises:
    HTTPException: If lockfile is malformed, system is invalid, or SBOM generation fails

Sends a `POST` request to `/api/v1/catalog/sbom/environment`

*/
    pub async fn environment_sbom_api_v1_catalog_sbom_environment_post<'a>(
        &'a self,
        format: Option<types::SbomFormat>,
        body: &'a types::EnvironmentSbomRequest,
    ) -> Result<
        ResponseValue<::serde_json::Map<::std::string::String, ::serde_json::Value>>,
        Error<types::ErrorResponse>,
    > {
        let url = format!("{}/api/v1/catalog/sbom/environment", self.baseurl,);
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .query(&progenitor_client::QueryParam::new("format", &format))
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "environment_sbom_api_v1_catalog_sbom_environment_post",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get SBOM for a package derivation

Get SBOM (Software Bill of Materials) for a package derivation.

Args:
    derivation: The derivation filename without /nix/store/ prefix
                (e.g., "abc123def-foo-1.0.drv")
    outputs: Optional comma-separated list of outputs (e.g., "out,bin,dev").
             If omitted, includes the package's default outputs from derivation metadata.
             Must be alphanumeric with underscores only, max 100 chars total.
    format: SBOM format (defaults to SbomFormat.SPDX_2_3_JSON)
    body: Request body (reserved for future extension)
    auth_result: Authentication payload
    cache: Request-scoped dependency cache (injected)

Returns:
    SBOM document in the requested format

Sends a `POST` request to `/api/v1/catalog/sbom/package/{derivation}`

Arguments:
- `derivation`
- `format`
- `outputs`: Comma-separated list of output names (e.g., 'out,bin,dev')
- `body`
*/
    pub async fn package_sbom_api_v1_catalog_sbom_package_derivation_post<'a>(
        &'a self,
        derivation: &'a str,
        format: Option<types::SbomFormat>,
        outputs: Option<&'a types::Outputs>,
        body: &'a types::PackageSbomRequest,
    ) -> Result<
        ResponseValue<::serde_json::Map<::std::string::String, ::serde_json::Value>>,
        Error<types::ErrorResponse>,
    > {
        let url = format!(
            "{}/api/v1/catalog/sbom/package/{}", self.baseurl, encode_path(& derivation
            .to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .query(&progenitor_client::QueryParam::new("format", &format))
            .query(&progenitor_client::QueryParam::new("outputs", &outputs))
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "package_sbom_api_v1_catalog_sbom_package_derivation_post",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Search for packages

Search the catalog(s) under the given criteria for matching packages.

Required Query Parameters:
- **system**: The system architecture to search for (e.g., x86_64-linux)

Optional Query Parameters:
- **search_term**: The search term to filter packages by
- **catalogs**: Comma separated list of catalog names to search; defaults to
  all catalogs. Note: when searching base catalog, search_term is required.
- **page**: Page number for pagination (default: 0)
- **pageSize**: Page size for pagination (default: 10)

Returns:
- **PackageSearchResult**: A list of PackageInfoSearch items and total count

Sends a `GET` request to `/api/v1/catalog/search`

*/
    pub async fn search_api_v1_catalog_search_get<'a>(
        &'a self,
        catalogs: Option<&'a str>,
        page: Option<i64>,
        page_size: Option<i64>,
        search_term: Option<&'a types::SearchTerm>,
        system: types::PackageSystem,
    ) -> Result<ResponseValue<types::PackageSearchResult>, Error<types::ErrorResponse>> {
        let url = format!("{}/api/v1/catalog/search", self.baseurl,);
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .query(&progenitor_client::QueryParam::new("catalogs", &catalogs))
            .query(&progenitor_client::QueryParam::new("page", &page))
            .query(&progenitor_client::QueryParam::new("pageSize", &page_size))
            .query(&progenitor_client::QueryParam::new("search_term", &search_term))
            .query(&progenitor_client::QueryParam::new("system", &system))
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "search_api_v1_catalog_search_get",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Adjust various settings

Adjusts various settings on the catalog service.

Query Parameters:
- **key**: The the key to adjust.
    - "plan" - Enables the logging of the DB query plan for queries for
    **value** seconds.  It will be scheduled to turn off automatically after
    that.

Sends a `POST` request to `/api/v1/catalog/settings/{key}`

*/
    pub async fn settings_api_v1_catalog_settings_key_post<'a>(
        &'a self,
        key: &'a str,
        value: &'a str,
    ) -> Result<ResponseValue<::serde_json::Value>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/settings/{}", self.baseurl, encode_path(& key
            .to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .query(&progenitor_client::QueryParam::new("value", &value))
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "settings_api_v1_catalog_settings_key_post",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get store info for a list of derivations

Get store info for a list of output paths.

Body Parameters:
- **StoreInfoRequest**: Object containing an `outpaths` list of output paths

Returns:
- **StoreInfoResponse**: A map of output path to a list of StoreInfo objects,
  each containing catalog, package, URL, and optional authentication info
  for downloading the binary artifacts.

Sends a `POST` request to `/api/v1/catalog/store`

*/
    pub async fn get_store_info_api_v1_catalog_store_post<'a>(
        &'a self,
        body: &'a types::StoreInfoRequest,
    ) -> Result<ResponseValue<types::StoreInfoResponse>, Error<types::ErrorResponse>> {
        let url = format!("{}/api/v1/catalog/store", self.baseurl,);
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "get_store_info_api_v1_catalog_store_post",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get status for a list of storepaths

Get status for a list of store paths.

Body Parameters:
- **StorepathRequest**: Object containing an `outpaths` list of store paths

Returns:
- **StorepathStatusResponse**: A map of store path to a list of
  StorepathStatus objects indicating catalog, package, and narinfo status.

Sends a `POST` request to `/api/v1/catalog/store/status`

*/
    pub async fn get_storepath_status_api_v1_catalog_store_status_post<'a>(
        &'a self,
        body: &'a types::StoreInfoRequest,
    ) -> Result<
        ResponseValue<types::StorepathStatusResponse>,
        Error<types::ErrorResponse>,
    > {
        let url = format!("{}/api/v1/catalog/store/status", self.baseurl,);
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                ::reqwest::header::ACCEPT,
                ::reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "get_storepath_status_api_v1_catalog_store_status_post",
        };
        match (async |request: &mut ::reqwest::Request| {
            if let Some(span) = ::sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    request
                        .headers_mut()
                        .append(k, ::reqwest::header::HeaderValue::from_str(&v)?);
                }
            }
            Ok::<_, Box<dyn ::std::error::Error>>(())
        })(&mut request)
            .await
        {
            Ok(_) => {}
            Err(e) => return Err(Error::Custom(e.to_string())),
        }
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
}
/// Items consumers will typically use such as the Client.
pub mod prelude {
    #[allow(unused_imports)]
    pub use super::Client;
}
