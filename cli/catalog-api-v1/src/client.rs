#[allow(unused_imports)]
pub use progenitor_client::{ByteStream, Error, ResponseValue};
#[allow(unused_imports)]
use progenitor_client::{encode_path, RequestBuilderExt};
#[allow(unused_imports)]
use reqwest::header::{HeaderMap, HeaderValue};
/// Types used as operation parameters and responses.
#[allow(clippy::all)]
pub mod types {
    use serde::{Deserialize, Serialize};
    #[allow(unused_imports)]
    use std::convert::TryFrom;
    /// Error types.
    pub mod error {
        /// Error from a TryFrom or FromStr implementation.
        pub struct ConversionError(std::borrow::Cow<'static, str>);
        impl std::error::Error for ConversionError {}
        impl std::fmt::Display for ConversionError {
            fn fmt(
                &self,
                f: &mut std::fmt::Formatter<'_>,
            ) -> Result<(), std::fmt::Error> {
                std::fmt::Display::fmt(&self.0, f)
            }
        }
        impl std::fmt::Debug for ConversionError {
            fn fmt(
                &self,
                f: &mut std::fmt::Formatter<'_>,
            ) -> Result<(), std::fmt::Error> {
                std::fmt::Debug::fmt(&self.0, f)
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
    ///CatalogPage
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "CatalogPage",
    ///  "examples": [
    ///    {
    ///      "description": "A very nice Item",
    ///      "license": "foo",
    ///      "locked_url": "git:git?rev=xyz",
    ///      "name": "curl",
    ///      "outputs": "{}",
    ///      "outputs_to_install": "{}",
    ///      "pkg_path": "foo.bar.curl",
    ///      "pname": "curl",
    ///      "rev": "xyz",
    ///      "rev_count": 4,
    ///      "rev_date": 0,
    ///      "search_string": "curl^curl^my description",
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
    ///    "page",
    ///    "url"
    ///  ],
    ///  "properties": {
    ///    "packages": {
    ///      "title": "Packages",
    ///      "anyOf": [
    ///        {
    ///          "type": "array",
    ///          "items": {
    ///            "$ref": "#/components/schemas/PackageResolutionInfo"
    ///          }
    ///        }
    ///      ]
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
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct CatalogPage {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub packages: Vec<PackageResolutionInfo>,
        pub page: i64,
        pub url: String,
    }
    impl From<&CatalogPage> for CatalogPage {
        fn from(value: &CatalogPage) -> Self {
            value.clone()
        }
    }
    ///CatalogStatus
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "CatalogStatus",
    ///  "type": "object",
    ///  "required": [
    ///    "attribute_path_ct",
    ///    "catalogs",
    ///    "derivations_ct",
    ///    "latest_rev",
    ///    "latest_scrape",
    ///    "pages_ct",
    ///    "schema_version",
    ///    "search_index_ct",
    ///    "systems"
    ///  ],
    ///  "properties": {
    ///    "attribute_path_ct": {
    ///      "title": "Attribute Path Ct",
    ///      "type": "integer"
    ///    },
    ///    "catalogs": {
    ///      "title": "Catalogs",
    ///      "type": "array",
    ///      "items": {
    ///        "type": "string"
    ///      }
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
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "pages_ct": {
    ///      "title": "Pages Ct",
    ///      "type": "integer"
    ///    },
    ///    "schema_version": {
    ///      "title": "Schema Version",
    ///      "type": "number"
    ///    },
    ///    "search_index_ct": {
    ///      "title": "Search Index Ct",
    ///      "type": "integer"
    ///    },
    ///    "systems": {
    ///      "title": "Systems",
    ///      "type": "array",
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct CatalogStatus {
        pub attribute_path_ct: i64,
        pub catalogs: Vec<String>,
        pub derivations_ct: i64,
        pub latest_rev: chrono::DateTime<chrono::offset::Utc>,
        pub latest_scrape: chrono::DateTime<chrono::offset::Utc>,
        pub pages_ct: i64,
        pub schema_version: f64,
        pub search_index_ct: i64,
        pub systems: Vec<String>,
    }
    impl From<&CatalogStatus> for CatalogStatus {
        fn from(value: &CatalogStatus) -> Self {
            value.clone()
        }
    }
    ///ErrorResponse
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ErrorResponse",
    ///  "type": "object",
    ///  "required": [
    ///    "detail",
    ///    "status_code"
    ///  ],
    ///  "properties": {
    ///    "detail": {
    ///      "title": "Detail",
    ///      "type": "string"
    ///    },
    ///    "status_code": {
    ///      "title": "Status Code",
    ///      "type": "integer"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct ErrorResponse {
        pub detail: String,
        pub status_code: i64,
    }
    impl From<&ErrorResponse> for ErrorResponse {
        fn from(value: &ErrorResponse) -> Self {
            value.clone()
        }
    }
    ///PackageDescriptor
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageDescriptor",
    ///  "examples": [
    ///    {
    ///      "name": "curl",
    ///      "pkgpath": "curl"
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "name",
    ///    "pkgpath"
    ///  ],
    ///  "properties": {
    ///    "derivation": {
    ///      "title": "Derivation",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "pkgpath": {
    ///      "title": "pkgpath",
    ///      "type": "string"
    ///    },
    ///    "semver": {
    ///      "title": "Semver",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "version": {
    ///      "title": "Version",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct PackageDescriptor {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub derivation: Option<String>,
        pub name: String,
        pub pkgpath: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub semver: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub version: Option<String>,
    }
    impl From<&PackageDescriptor> for PackageDescriptor {
        fn from(value: &PackageDescriptor) -> Self {
            value.clone()
        }
    }
    ///PackageGroup
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
    ///          "name": "curl",
    ///          "pkgpath": "curl"
    ///        },
    ///        {
    ///          "name": "slack",
    ///          "pkgpath": "slack"
    ///        },
    ///        {
    ///          "name": "xeyes",
    ///          "pkgpath": "xorg.xeyes"
    ///        }
    ///      ],
    ///      "name": "test",
    ///      "system": "x86_64-linux"
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "descriptors",
    ///    "name",
    ///    "system"
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
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "system": {
    ///      "$ref": "#/components/schemas/SystemEnum"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct PackageGroup {
        pub descriptors: Vec<PackageDescriptor>,
        pub name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub stability: Option<String>,
        pub system: SystemEnum,
    }
    impl From<&PackageGroup> for PackageGroup {
        fn from(value: &PackageGroup) -> Self {
            value.clone()
        }
    }
    ///PackageGroups
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
    ///              "name": "curl",
    ///              "pkgpath": "curl"
    ///            },
    ///            {
    ///              "name": "slack",
    ///              "pkgpath": "slack"
    ///            },
    ///            {
    ///              "name": "xeyes",
    ///              "pkgpath": "xorg.xeyes"
    ///            }
    ///          ],
    ///          "name": "test",
    ///          "system": "x86_64-linux"
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
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct PackageGroups {
        pub items: Vec<PackageGroup>,
    }
    impl From<&PackageGroups> for PackageGroups {
        fn from(value: &PackageGroups) -> Self {
            value.clone()
        }
    }
    ///PackageInfoApiInput
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageInfoAPI",
    ///  "examples": [
    ///    {
    ///      "description": "A very nice Item",
    ///      "license": "foo",
    ///      "locked_url": "git:git?rev=xyz",
    ///      "name": "curl",
    ///      "outputs": "{}",
    ///      "outputs_to_install": "{}",
    ///      "pkg_path": "foo.bar.curl",
    ///      "pname": "curl",
    ///      "rev": "xyz",
    ///      "rev_count": 4,
    ///      "rev_date": 0,
    ///      "search_string": "curl^curl^my description",
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
    ///    "description",
    ///    "license",
    ///    "locked_url",
    ///    "name",
    ///    "outputs",
    ///    "outputs_to_install",
    ///    "pname",
    ///    "rev",
    ///    "rev_count",
    ///    "rev_date",
    ///    "stabilities",
    ///    "system",
    ///    "version"
    ///  ],
    ///  "properties": {
    ///    "attr_path": {
    ///      "title": "Attr Path",
    ///      "type": "string"
    ///    },
    ///    "description": {
    ///      "title": "Description",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "license": {
    ///      "title": "License",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "locked_url": {
    ///      "title": "Locked Url",
    ///      "type": "string"
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "outputs": {
    ///      "title": "Outputs",
    ///      "anyOf": [
    ///        {
    ///          "type": "object"
    ///        }
    ///      ]
    ///    },
    ///    "outputs_to_install": {
    ///      "title": "Outputs To Install",
    ///      "anyOf": [
    ///        {
    ///          "type": "array",
    ///          "items": {}
    ///        }
    ///      ]
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
    ///    "stabilities": {
    ///      "title": "Stabilities",
    ///      "type": "array",
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "system": {
    ///      "$ref": "#/components/schemas/SystemEnum"
    ///    },
    ///    "version": {
    ///      "title": "Version",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct PackageInfoApiInput {
        pub attr_path: String,
        pub description: String,
        pub license: String,
        pub locked_url: String,
        pub name: String,
        pub outputs: serde_json::Map<String, serde_json::Value>,
        pub outputs_to_install: Vec<serde_json::Value>,
        pub pname: String,
        pub rev: String,
        pub rev_count: i64,
        pub rev_date: chrono::DateTime<chrono::offset::Utc>,
        pub stabilities: Vec<String>,
        pub system: SystemEnum,
        pub version: String,
    }
    impl From<&PackageInfoApiInput> for PackageInfoApiInput {
        fn from(value: &PackageInfoApiInput) -> Self {
            value.clone()
        }
    }
    ///PackageInfoApiOutput
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageInfoAPI",
    ///  "examples": [
    ///    {
    ///      "description": "A very nice Item",
    ///      "license": "foo",
    ///      "locked_url": "git:git?rev=xyz",
    ///      "name": "curl",
    ///      "outputs": "{}",
    ///      "outputs_to_install": "{}",
    ///      "pkg_path": "foo.bar.curl",
    ///      "pname": "curl",
    ///      "rev": "xyz",
    ///      "rev_count": 4,
    ///      "rev_date": 0,
    ///      "search_string": "curl^curl^my description",
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
    ///    "description",
    ///    "license",
    ///    "locked_url",
    ///    "name",
    ///    "outputs",
    ///    "outputs_to_install",
    ///    "pkg_path",
    ///    "pname",
    ///    "rev",
    ///    "rev_count",
    ///    "rev_date",
    ///    "stabilities",
    ///    "system",
    ///    "version"
    ///  ],
    ///  "properties": {
    ///    "description": {
    ///      "title": "Description",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "license": {
    ///      "title": "License",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "locked_url": {
    ///      "title": "Locked Url",
    ///      "type": "string"
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "outputs": {
    ///      "title": "Outputs",
    ///      "anyOf": [
    ///        {
    ///          "type": "object"
    ///        }
    ///      ]
    ///    },
    ///    "outputs_to_install": {
    ///      "title": "Outputs To Install",
    ///      "anyOf": [
    ///        {
    ///          "type": "array",
    ///          "items": {}
    ///        }
    ///      ]
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
    ///    "stabilities": {
    ///      "title": "Stabilities",
    ///      "type": "array",
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "system": {
    ///      "$ref": "#/components/schemas/SystemEnum"
    ///    },
    ///    "version": {
    ///      "title": "Version",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct PackageInfoApiOutput {
        pub description: String,
        pub license: String,
        pub locked_url: String,
        pub name: String,
        pub outputs: serde_json::Map<String, serde_json::Value>,
        pub outputs_to_install: Vec<serde_json::Value>,
        pub pkg_path: String,
        pub pname: String,
        pub rev: String,
        pub rev_count: i64,
        pub rev_date: chrono::DateTime<chrono::offset::Utc>,
        pub stabilities: Vec<String>,
        pub system: SystemEnum,
        pub version: String,
    }
    impl From<&PackageInfoApiOutput> for PackageInfoApiOutput {
        fn from(value: &PackageInfoApiOutput) -> Self {
            value.clone()
        }
    }
    ///PackageInfoCommonInput
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageInfoCommon",
    ///  "type": "object",
    ///  "required": [
    ///    "attr_path",
    ///    "description",
    ///    "license",
    ///    "name",
    ///    "outputs",
    ///    "outputs_to_install",
    ///    "pname",
    ///    "rev",
    ///    "rev_count",
    ///    "rev_date",
    ///    "system",
    ///    "version"
    ///  ],
    ///  "properties": {
    ///    "attr_path": {
    ///      "title": "Attr Path",
    ///      "type": "string"
    ///    },
    ///    "description": {
    ///      "title": "Description",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "license": {
    ///      "title": "License",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "outputs": {
    ///      "title": "Outputs",
    ///      "anyOf": [
    ///        {
    ///          "type": "object"
    ///        }
    ///      ]
    ///    },
    ///    "outputs_to_install": {
    ///      "title": "Outputs To Install",
    ///      "anyOf": [
    ///        {
    ///          "type": "array",
    ///          "items": {}
    ///        }
    ///      ]
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
    ///    "system": {
    ///      "$ref": "#/components/schemas/SystemEnum"
    ///    },
    ///    "version": {
    ///      "title": "Version",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct PackageInfoCommonInput {
        pub attr_path: String,
        pub description: String,
        pub license: String,
        pub name: String,
        pub outputs: serde_json::Map<String, serde_json::Value>,
        pub outputs_to_install: Vec<serde_json::Value>,
        pub pname: String,
        pub rev: String,
        pub rev_count: i64,
        pub rev_date: chrono::DateTime<chrono::offset::Utc>,
        pub system: SystemEnum,
        pub version: String,
    }
    impl From<&PackageInfoCommonInput> for PackageInfoCommonInput {
        fn from(value: &PackageInfoCommonInput) -> Self {
            value.clone()
        }
    }
    ///PackageInfoCommonOutput
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageInfoCommon",
    ///  "type": "object",
    ///  "required": [
    ///    "description",
    ///    "license",
    ///    "name",
    ///    "outputs",
    ///    "outputs_to_install",
    ///    "pkg_path",
    ///    "pname",
    ///    "rev",
    ///    "rev_count",
    ///    "rev_date",
    ///    "system",
    ///    "version"
    ///  ],
    ///  "properties": {
    ///    "description": {
    ///      "title": "Description",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "license": {
    ///      "title": "License",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "outputs": {
    ///      "title": "Outputs",
    ///      "anyOf": [
    ///        {
    ///          "type": "object"
    ///        }
    ///      ]
    ///    },
    ///    "outputs_to_install": {
    ///      "title": "Outputs To Install",
    ///      "anyOf": [
    ///        {
    ///          "type": "array",
    ///          "items": {}
    ///        }
    ///      ]
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
    ///    "system": {
    ///      "$ref": "#/components/schemas/SystemEnum"
    ///    },
    ///    "version": {
    ///      "title": "Version",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct PackageInfoCommonOutput {
        pub description: String,
        pub license: String,
        pub name: String,
        pub outputs: serde_json::Map<String, serde_json::Value>,
        pub outputs_to_install: Vec<serde_json::Value>,
        pub pkg_path: String,
        pub pname: String,
        pub rev: String,
        pub rev_count: i64,
        pub rev_date: chrono::DateTime<chrono::offset::Utc>,
        pub system: SystemEnum,
        pub version: String,
    }
    impl From<&PackageInfoCommonOutput> for PackageInfoCommonOutput {
        fn from(value: &PackageInfoCommonOutput) -> Self {
            value.clone()
        }
    }
    ///PackageResolutionInfo
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
    ///    "license",
    ///    "locked_url",
    ///    "name",
    ///    "outputs",
    ///    "outputs_to_install",
    ///    "pname",
    ///    "rev",
    ///    "rev_count",
    ///    "rev_date",
    ///    "scrape_date",
    ///    "stabilities",
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
    ///      "type": "boolean"
    ///    },
    ///    "derivation": {
    ///      "title": "Derivation",
    ///      "type": "string"
    ///    },
    ///    "description": {
    ///      "title": "Description",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "license": {
    ///      "title": "License",
    ///      "anyOf": [
    ///        {
    ///          "type": "string"
    ///        }
    ///      ]
    ///    },
    ///    "locked_url": {
    ///      "title": "Locked Url",
    ///      "type": "string"
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "outputs": {
    ///      "title": "Outputs",
    ///      "anyOf": [
    ///        {
    ///          "type": "object"
    ///        }
    ///      ]
    ///    },
    ///    "outputs_to_install": {
    ///      "title": "Outputs To Install",
    ///      "anyOf": [
    ///        {
    ///          "type": "array",
    ///          "items": {}
    ///        }
    ///      ]
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
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "stabilities": {
    ///      "title": "Stabilities",
    ///      "anyOf": [
    ///        {
    ///          "type": "array",
    ///          "items": {
    ///            "type": "string"
    ///          }
    ///        }
    ///      ]
    ///    },
    ///    "unfree": {
    ///      "title": "Unfree",
    ///      "anyOf": [
    ///        {
    ///          "type": "boolean"
    ///        }
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
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct PackageResolutionInfo {
        pub attr_path: String,
        pub broken: bool,
        pub derivation: String,
        pub description: String,
        pub license: String,
        pub locked_url: String,
        pub name: String,
        pub outputs: serde_json::Map<String, serde_json::Value>,
        pub outputs_to_install: Vec<serde_json::Value>,
        pub pname: String,
        pub rev: String,
        pub rev_count: i64,
        pub rev_date: chrono::DateTime<chrono::offset::Utc>,
        pub scrape_date: chrono::DateTime<chrono::offset::Utc>,
        pub stabilities: Vec<String>,
        pub unfree: bool,
        pub version: String,
    }
    impl From<&PackageResolutionInfo> for PackageResolutionInfo {
        fn from(value: &PackageResolutionInfo) -> Self {
            value.clone()
        }
    }
    ///PackageSearchResultInput
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageSearchResult",
    ///  "examples": [
    ///    [
    ///      {
    ///        "description": "A very nice Item",
    ///        "license": "foo",
    ///        "locked_url": "git:git?rev=xyz",
    ///        "name": "curl",
    ///        "outputs": "{}",
    ///        "outputs_to_install": "{}",
    ///        "pkg_path": "foo.bar.curl",
    ///        "pname": "curl",
    ///        "rev": "xyz",
    ///        "rev_count": 4,
    ///        "rev_date": 0,
    ///        "search_string": "curl^curl^my description",
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
    ///        "$ref": "#/components/schemas/PackageInfoAPI-Input"
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
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct PackageSearchResultInput {
        pub items: Vec<PackageInfoApiInput>,
        pub total_count: i64,
    }
    impl From<&PackageSearchResultInput> for PackageSearchResultInput {
        fn from(value: &PackageSearchResultInput) -> Self {
            value.clone()
        }
    }
    ///PackageSearchResultOutput
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageSearchResult",
    ///  "examples": [
    ///    [
    ///      {
    ///        "description": "A very nice Item",
    ///        "license": "foo",
    ///        "locked_url": "git:git?rev=xyz",
    ///        "name": "curl",
    ///        "outputs": "{}",
    ///        "outputs_to_install": "{}",
    ///        "pkg_path": "foo.bar.curl",
    ///        "pname": "curl",
    ///        "rev": "xyz",
    ///        "rev_count": 4,
    ///        "rev_date": 0,
    ///        "search_string": "curl^curl^my description",
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
    ///        "$ref": "#/components/schemas/PackageInfoAPI-Output"
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
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct PackageSearchResultOutput {
        pub items: Vec<PackageInfoApiOutput>,
        pub total_count: i64,
    }
    impl From<&PackageSearchResultOutput> for PackageSearchResultOutput {
        fn from(value: &PackageSearchResultOutput) -> Self {
            value.clone()
        }
    }
    ///PackagesResultInput
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
    ///        "$ref": "#/components/schemas/PackageInfoCommon-Input"
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
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct PackagesResultInput {
        pub items: Vec<PackageInfoCommonInput>,
        pub total_count: i64,
    }
    impl From<&PackagesResultInput> for PackagesResultInput {
        fn from(value: &PackagesResultInput) -> Self {
            value.clone()
        }
    }
    ///PackagesResultOutput
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
    ///        "$ref": "#/components/schemas/PackageInfoCommon-Output"
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
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct PackagesResultOutput {
        pub items: Vec<PackageInfoCommonOutput>,
        pub total_count: i64,
    }
    impl From<&PackagesResultOutput> for PackagesResultOutput {
        fn from(value: &PackagesResultOutput) -> Self {
            value.clone()
        }
    }
    ///ResolvedPackageGroupInput
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ResolvedPackageGroup",
    ///  "examples": [
    ///    {
    ///      "description": "A very nice Item",
    ///      "license": "foo",
    ///      "locked_url": "git:git?rev=xyz",
    ///      "name": "curl",
    ///      "outputs": "{}",
    ///      "outputs_to_install": "{}",
    ///      "pkg_path": "foo.bar.curl",
    ///      "pname": "curl",
    ///      "rev": "xyz",
    ///      "rev_count": 4,
    ///      "rev_date": 0,
    ///      "search_string": "curl^curl^my description",
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
    ///    "name",
    ///    "pages",
    ///    "system"
    ///  ],
    ///  "properties": {
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "pages": {
    ///      "title": "Pages",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/CatalogPage"
    ///      }
    ///    },
    ///    "system": {
    ///      "$ref": "#/components/schemas/SystemEnum"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct ResolvedPackageGroupInput {
        pub name: String,
        pub pages: Vec<CatalogPage>,
        pub system: SystemEnum,
    }
    impl From<&ResolvedPackageGroupInput> for ResolvedPackageGroupInput {
        fn from(value: &ResolvedPackageGroupInput) -> Self {
            value.clone()
        }
    }
    ///ResolvedPackageGroupOutput
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ResolvedPackageGroup",
    ///  "examples": [
    ///    {
    ///      "description": "A very nice Item",
    ///      "license": "foo",
    ///      "locked_url": "git:git?rev=xyz",
    ///      "name": "curl",
    ///      "outputs": "{}",
    ///      "outputs_to_install": "{}",
    ///      "pkg_path": "foo.bar.curl",
    ///      "pname": "curl",
    ///      "rev": "xyz",
    ///      "rev_count": 4,
    ///      "rev_date": 0,
    ///      "search_string": "curl^curl^my description",
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
    ///    "name",
    ///    "pages",
    ///    "system"
    ///  ],
    ///  "properties": {
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "pages": {
    ///      "title": "Pages",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/CatalogPage"
    ///      }
    ///    },
    ///    "system": {
    ///      "$ref": "#/components/schemas/SystemEnum"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct ResolvedPackageGroupOutput {
        pub name: String,
        pub pages: Vec<CatalogPage>,
        pub system: SystemEnum,
    }
    impl From<&ResolvedPackageGroupOutput> for ResolvedPackageGroupOutput {
        fn from(value: &ResolvedPackageGroupOutput) -> Self {
            value.clone()
        }
    }
    ///ResolvedPackageGroupsInput
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
    ///        "$ref": "#/components/schemas/ResolvedPackageGroup-Input"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct ResolvedPackageGroupsInput {
        pub items: Vec<ResolvedPackageGroupInput>,
    }
    impl From<&ResolvedPackageGroupsInput> for ResolvedPackageGroupsInput {
        fn from(value: &ResolvedPackageGroupsInput) -> Self {
            value.clone()
        }
    }
    ///ResolvedPackageGroupsOutput
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
    ///        "$ref": "#/components/schemas/ResolvedPackageGroup-Output"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct ResolvedPackageGroupsOutput {
        pub items: Vec<ResolvedPackageGroupOutput>,
    }
    impl From<&ResolvedPackageGroupsOutput> for ResolvedPackageGroupsOutput {
        fn from(value: &ResolvedPackageGroupsOutput) -> Self {
            value.clone()
        }
    }
    ///SearchTerm
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "Search Term",
    ///  "type": "string",
    ///  "pattern": "[a-zA-Z0-9\\-\\._,]{2,200}"
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
    pub struct SearchTerm(String);
    impl std::ops::Deref for SearchTerm {
        type Target = String;
        fn deref(&self) -> &String {
            &self.0
        }
    }
    impl From<SearchTerm> for String {
        fn from(value: SearchTerm) -> Self {
            value.0
        }
    }
    impl From<&SearchTerm> for SearchTerm {
        fn from(value: &SearchTerm) -> Self {
            value.clone()
        }
    }
    impl std::str::FromStr for SearchTerm {
        type Err = self::error::ConversionError;
        fn from_str(value: &str) -> Result<Self, self::error::ConversionError> {
            if regress::Regex::new("[a-zA-Z0-9\\-\\._,]{2,200}")
                .unwrap()
                .find(value)
                .is_none()
            {
                return Err(
                    "doesn't match pattern \"[a-zA-Z0-9\\-\\._,]{2,200}\"".into(),
                );
            }
            Ok(Self(value.to_string()))
        }
    }
    impl std::convert::TryFrom<&str> for SearchTerm {
        type Error = self::error::ConversionError;
        fn try_from(value: &str) -> Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl std::convert::TryFrom<&String> for SearchTerm {
        type Error = self::error::ConversionError;
        fn try_from(value: &String) -> Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl std::convert::TryFrom<String> for SearchTerm {
        type Error = self::error::ConversionError;
        fn try_from(value: String) -> Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl<'de> serde::Deserialize<'de> for SearchTerm {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            String::deserialize(deserializer)?
                .parse()
                .map_err(|e: self::error::ConversionError| {
                    <D::Error as serde::de::Error>::custom(e.to_string())
                })
        }
    }
    ///SystemEnum
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "SystemEnum",
    ///  "type": "string",
    ///  "enum": [
    ///    "aarch64-darwin",
    ///    "aarch64-linux",
    ///    "x86_64-darwin",
    ///    "x86_64-linux"
    ///  ]
    ///}
    /// ```
    /// </details>
    #[derive(
        Clone,
        Copy,
        Debug,
        Deserialize,
        Eq,
        Hash,
        Ord,
        PartialEq,
        PartialOrd,
        Serialize
    )]
    pub enum SystemEnum {
        #[serde(rename = "aarch64-darwin")]
        Aarch64Darwin,
        #[serde(rename = "aarch64-linux")]
        Aarch64Linux,
        #[serde(rename = "x86_64-darwin")]
        X8664Darwin,
        #[serde(rename = "x86_64-linux")]
        X8664Linux,
    }
    impl From<&SystemEnum> for SystemEnum {
        fn from(value: &SystemEnum) -> Self {
            value.clone()
        }
    }
    impl ToString for SystemEnum {
        fn to_string(&self) -> String {
            match *self {
                Self::Aarch64Darwin => "aarch64-darwin".to_string(),
                Self::Aarch64Linux => "aarch64-linux".to_string(),
                Self::X8664Darwin => "x86_64-darwin".to_string(),
                Self::X8664Linux => "x86_64-linux".to_string(),
            }
        }
    }
    impl std::str::FromStr for SystemEnum {
        type Err = self::error::ConversionError;
        fn from_str(value: &str) -> Result<Self, self::error::ConversionError> {
            match value {
                "aarch64-darwin" => Ok(Self::Aarch64Darwin),
                "aarch64-linux" => Ok(Self::Aarch64Linux),
                "x86_64-darwin" => Ok(Self::X8664Darwin),
                "x86_64-linux" => Ok(Self::X8664Linux),
                _ => Err("invalid value".into()),
            }
        }
    }
    impl std::convert::TryFrom<&str> for SystemEnum {
        type Error = self::error::ConversionError;
        fn try_from(value: &str) -> Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl std::convert::TryFrom<&String> for SystemEnum {
        type Error = self::error::ConversionError;
        fn try_from(value: &String) -> Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl std::convert::TryFrom<String> for SystemEnum {
        type Error = self::error::ConversionError;
        fn try_from(value: String) -> Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
}
#[derive(Clone, Debug)]
/**Client for Flox Catalog Service


# Flox Catalog Service API

TBD

*Markdown is available here*


Version: v0.1.dev127+g74c9441.d19800101*/
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
            let dur = std::time::Duration::from_secs(15);
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
    /// Get the base URL to which requests are made.
    pub fn baseurl(&self) -> &String {
        &self.baseurl
    }
    /// Get the internal `reqwest::Client` used to make requests.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
    /// Get the version of this API.
    ///
    /// This string is pulled directly from the source OpenAPI
    /// document and may be in any format the API selects.
    pub fn api_version(&self) -> &'static str {
        "v0.1.dev127+g74c9441.d19800101"
    }
}
#[allow(clippy::all)]
impl Client {
    /**Search for packages

Search the catalog(s) under the given criteria for matching packages.

Required Query Parameters:
- **seach_term**: The search term to search on.
- **system**: The search will be constrained to packages on this system.

Optional Query Parameters:
- **catalogs**: Comma separated list of catalog names to search
- **page**: Optional page number for pagination (def = 0)
- **pageSize**: Optional page size for pagination (def = 10)

Returns:
- **PackageSearchResult**: A list of PackageInfo and the total result count

Sends a `GET` request to `/api/v1/catalog/search`

*/
    pub async fn search_api_v1_catalog_search_get<'a>(
        &'a self,
        catalogs: Option<&'a str>,
        page: Option<i64>,
        page_size: Option<i64>,
        search_term: &'a types::SearchTerm,
        system: types::SystemEnum,
    ) -> Result<
        ResponseValue<types::PackageSearchResultInput>,
        Error<types::ErrorResponse>,
    > {
        let url = format!("{}/api/v1/catalog/search", self.baseurl,);
        let mut query = Vec::with_capacity(5usize);
        if let Some(v) = &catalogs {
            query.push(("catalogs", v.to_string()));
        }
        if let Some(v) = &page {
            query.push(("page", v.to_string()));
        }
        if let Some(v) = &page_size {
            query.push(("pageSize", v.to_string()));
        }
        query.push(("search_term", search_term.to_string()));
        query.push(("system", system.to_string()));
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/json"),
            )
            .query(&query)
            .build()?;
        let result = self.client.execute(request).await;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Shows avaliable packages of a specfic package

Returns a list of versions for a given pkg-path

Required Query Parameters:
- **pkgpath**: The pkg-path, must be valid.

Optional Query Parameters:
- **page**: Optional page number for pagination (def = 0)
- **pageSize**: Optional page size for pagination (def = 10)

Returns:
- **PackageSearchResult**: A list of PackageInfo and the total result count

Sends a `GET` request to `/api/v1/catalog/packages/{pkgpath}`

*/
    pub async fn packages_api_v1_catalog_packages_pkgpath_get<'a>(
        &'a self,
        pkgpath: &'a str,
        page: Option<i64>,
        page_size: Option<i64>,
    ) -> Result<ResponseValue<types::PackagesResultInput>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/catalog/packages/{}", self.baseurl, encode_path(& pkgpath
            .to_string()),
        );
        let mut query = Vec::with_capacity(2usize);
        if let Some(v) = &page {
            query.push(("page", v.to_string()));
        }
        if let Some(v) = &page_size {
            query.push(("pageSize", v.to_string()));
        }
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/json"),
            )
            .query(&query)
            .build()?;
        let result = self.client.execute(request).await;
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
- **none**

Returns:
- **ResolvedPackageGroups**: A object with an `items` array of
    `ResolvedPackageGroup` items.

Resolution Rules:
- Each `PackageGroup` is resolved independently.
- Each page that has a package that meets each of the descriptors in that group is returned in the results
- The latest page will include details for each package in the group from that page
- The remainder pages are returned without details (to get those details... TBD)

A Package Descriptor match:
- **name**: [required] - is not used in matching, only for reference (TBD is
            there a uniqueness constraint?)
- **pkgpath**: [required] - this must match the nix attribute path exactly and in full
- **semver**: [optional] - This can be any valid semver range, and if given
    will attempt to parse the nix `version` field.  If it can and it is
    within the range, this check passes.  If it cannot parse `version` as a
    valid semver, or it is not within the range, it is exluded.
    - **allow-pre-release**: [optional] - Defaults to False.  Only applies
        when a **semver** constraint is given.  If true, a `version` that can
        be parsed as a valid semver, that includes a pre-release suffix will
        be included as a candidate.  Otherwise, they will be excluded.
- **version**: [optional] - If given, this must match the nix `version`
    field precisely. This overrides **semver** matching if provided.

Sends a `POST` request to `/api/v1/catalog/resolve`

*/
    pub async fn resolve_api_v1_catalog_resolve_post<'a>(
        &'a self,
        body: &'a types::PackageGroups,
    ) -> Result<
        ResponseValue<types::ResolvedPackageGroupsInput>,
        Error<types::ErrorResponse>,
    > {
        let url = format!("{}/api/v1/catalog/resolve", self.baseurl,);
        #[allow(unused_mut)]
        let mut request = self
            .client
            .post(url)
            .header(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/json"),
            )
            .json(&body)
            .build()?;
        let result = self.client.execute(request).await;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            406u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get basic catalog database status

Gather some basic status values from the database.

Returns:
- **CatalogStatus**: A dictionary of various status values.

Sends a `GET` request to `/api/v1/metrics/status`

*/
    pub async fn get_status_api_v1_metrics_status_get<'a>(
        &'a self,
    ) -> Result<ResponseValue<types::CatalogStatus>, Error<()>> {
        let url = format!("{}/api/v1/metrics/status", self.baseurl,);
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(url)
            .header(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/json"),
            )
            .build()?;
        let result = self.client.execute(request).await;
        let response = result?;
        match response.status().as_u16() {
            200u16 => ResponseValue::from_response(response).await,
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
}
/// Items consumers will typically use such as the Client.
pub mod prelude {
    #[allow(unused_imports)]
    pub use super::Client;
}
