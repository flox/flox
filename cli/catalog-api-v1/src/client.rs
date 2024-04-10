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
    ///      "attr_path": "foo.bar.curl",
    ///      "description": "A very nice Item",
    ///      "license": "foo",
    ///      "locked_url": "git:git?rev=xyz",
    ///      "name": "curl",
    ///      "outputs": "{}",
    ///      "outputs_to_install": "{}",
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
    ///HttpValidationError
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "HTTPValidationError",
    ///  "type": "object",
    ///  "properties": {
    ///    "detail": {
    ///      "title": "Detail",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/ValidationError"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct HttpValidationError {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub detail: Vec<ValidationError>,
    }
    impl From<&HttpValidationError> for HttpValidationError {
        fn from(value: &HttpValidationError) -> Self {
            value.clone()
        }
    }
    ///LocationItem
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "anyOf": [
    ///    {
    ///      "type": "string"
    ///    },
    ///    {
    ///      "type": "integer"
    ///    }
    ///  ]
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(untagged)]
    pub enum LocationItem {
        Variant0(String),
        Variant1(i64),
    }
    impl From<&LocationItem> for LocationItem {
        fn from(value: &LocationItem) -> Self {
            value.clone()
        }
    }
    impl std::str::FromStr for LocationItem {
        type Err = self::error::ConversionError;
        fn from_str(value: &str) -> Result<Self, self::error::ConversionError> {
            if let Ok(v) = value.parse() {
                Ok(Self::Variant0(v))
            } else if let Ok(v) = value.parse() {
                Ok(Self::Variant1(v))
            } else {
                Err("string conversion failed for all variants".into())
            }
        }
    }
    impl std::convert::TryFrom<&str> for LocationItem {
        type Error = self::error::ConversionError;
        fn try_from(value: &str) -> Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl std::convert::TryFrom<&String> for LocationItem {
        type Error = self::error::ConversionError;
        fn try_from(value: &String) -> Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl std::convert::TryFrom<String> for LocationItem {
        type Error = self::error::ConversionError;
        fn try_from(value: String) -> Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ToString for LocationItem {
        fn to_string(&self) -> String {
            match self {
                Self::Variant0(x) => x.to_string(),
                Self::Variant1(x) => x.to_string(),
            }
        }
    }
    impl From<i64> for LocationItem {
        fn from(value: i64) -> Self {
            Self::Variant1(value)
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
    ///      "name": "hello",
    ///      "pkgPath": "hello"
    ///    }
    ///  ],
    ///  "type": "object",
    ///  "required": [
    ///    "name",
    ///    "pkgPath"
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
    ///    "pkgPath": {
    ///      "title": "Pkgpath",
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
        #[serde(rename = "pkgPath")]
        pub pkg_path: String,
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
    ///          "name": "hello",
    ///          "pkgPath": "hello"
    ///        },
    ///        {
    ///          "name": "curl",
    ///          "pkgPath": "curl"
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
    ///              "name": "hello",
    ///              "pkgPath": "hello"
    ///            },
    ///            {
    ///              "name": "curl",
    ///              "pkgPath": "curl"
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
    ///PackageInfoApi
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PackageInfoAPI",
    ///  "examples": [
    ///    {
    ///      "attr_path": "foo.bar.curl",
    ///      "description": "A very nice Item",
    ///      "license": "foo",
    ///      "locked_url": "git:git?rev=xyz",
    ///      "name": "curl",
    ///      "outputs": "{}",
    ///      "outputs_to_install": "{}",
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
    ///      "type": "string"
    ///    },
    ///    "license": {
    ///      "title": "License",
    ///      "type": "string"
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
    ///      "type": "integer"
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
    pub struct PackageInfoApi {
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
        pub rev_date: i64,
        pub stabilities: Vec<String>,
        pub system: SystemEnum,
        pub version: String,
    }
    impl From<&PackageInfoApi> for PackageInfoApi {
        fn from(value: &PackageInfoApi) -> Self {
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
    ///      "type": "string"
    ///    },
    ///    "license": {
    ///      "title": "License",
    ///      "type": "string"
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
    ///      "type": "integer"
    ///    },
    ///    "scrape_date": {
    ///      "title": "Scrape Date",
    ///      "type": "integer"
    ///    },
    ///    "stabilities": {
    ///      "title": "Stabilities",
    ///      "type": "array",
    ///      "items": {
    ///        "type": "string"
    ///      }
    ///    },
    ///    "unfree": {
    ///      "title": "Unfree",
    ///      "type": "boolean"
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
        pub rev_date: i64,
        pub scrape_date: i64,
        pub stabilities: Vec<String>,
        pub unfree: bool,
        pub version: String,
    }
    impl From<&PackageResolutionInfo> for PackageResolutionInfo {
        fn from(value: &PackageResolutionInfo) -> Self {
            value.clone()
        }
    }
    ///PackageSearchResult
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
    ///        "description": "A very nice Item",
    ///        "license": "foo",
    ///        "locked_url": "git:git?rev=xyz",
    ///        "name": "curl",
    ///        "outputs": "{}",
    ///        "outputs_to_install": "{}",
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
    ///        "$ref": "#/components/schemas/PackageInfoAPI"
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
    pub struct PackageSearchResult {
        pub items: Vec<PackageInfoApi>,
        pub total_count: i64,
    }
    impl From<&PackageSearchResult> for PackageSearchResult {
        fn from(value: &PackageSearchResult) -> Self {
            value.clone()
        }
    }
    ///ResolvedPackageGroup
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ResolvedPackageGroup",
    ///  "examples": [
    ///    {
    ///      "attr_path": "foo.bar.curl",
    ///      "description": "A very nice Item",
    ///      "license": "foo",
    ///      "locked_url": "git:git?rev=xyz",
    ///      "name": "curl",
    ///      "outputs": "{}",
    ///      "outputs_to_install": "{}",
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
    pub struct ResolvedPackageGroup {
        pub name: String,
        pub pages: Vec<CatalogPage>,
        pub system: SystemEnum,
    }
    impl From<&ResolvedPackageGroup> for ResolvedPackageGroup {
        fn from(value: &ResolvedPackageGroup) -> Self {
            value.clone()
        }
    }
    ///ResolvedPackageGroups
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
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct ResolvedPackageGroups {
        pub items: Vec<ResolvedPackageGroup>,
    }
    impl From<&ResolvedPackageGroups> for ResolvedPackageGroups {
        fn from(value: &ResolvedPackageGroups) -> Self {
            value.clone()
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
    ///ValidationError
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ValidationError",
    ///  "type": "object",
    ///  "required": [
    ///    "loc",
    ///    "msg",
    ///    "type"
    ///  ],
    ///  "properties": {
    ///    "loc": {
    ///      "title": "Location",
    ///      "type": "array",
    ///      "items": {
    ///        "anyOf": [
    ///          {
    ///            "type": "string"
    ///          },
    ///          {
    ///            "type": "integer"
    ///          }
    ///        ]
    ///      }
    ///    },
    ///    "msg": {
    ///      "title": "Message",
    ///      "type": "string"
    ///    },
    ///    "type": {
    ///      "title": "Error Type",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct ValidationError {
        pub loc: Vec<LocationItem>,
        pub msg: String,
        #[serde(rename = "type")]
        pub type_: String,
    }
    impl From<&ValidationError> for ValidationError {
        fn from(value: &ValidationError) -> Self {
            value.clone()
        }
    }
}
#[derive(Clone, Debug)]
/**Client for Flox Catalog Server


# Flox Catalog API

## Markdown

Section

## More markdown

You will be able to:

- **Search** for packages


Version: v1*/
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
        "v1"
    }
}
#[allow(clippy::all)]
impl Client {
    /**Search for packages

Search the catalog(s) under the given criteria for matching packages.

Required Query Parameters:
- **name**: _description_
- **system**: _description_

Optional Query Parameters:
- **catalogs**: Comma separated list of catalog names to search
- **page**: _description_
- **pageSize**: _description_

Returns:
- **PackageSearchResult**: _description_

Sends a `GET` request to `/api/v1/catalog/search`

*/
    pub async fn search_api_v1_catalog_search_get<'a>(
        &'a self,
        catalogs: &'a str,
        name: &'a str,
        page: Option<i64>,
        page_size: Option<i64>,
        system: types::SystemEnum,
    ) -> Result<
        ResponseValue<types::PackageSearchResult>,
        Error<types::HttpValidationError>,
    > {
        let url = format!("{}/api/v1/catalog/search", self.baseurl,);
        let mut query = Vec::with_capacity(5usize);
        query.push(("catalogs", catalogs.to_string()));
        query.push(("name", name.to_string()));
        if let Some(v) = &page {
            query.push(("page", v.to_string()));
        }
        if let Some(v) = &page_size {
            query.push(("pageSize", v.to_string()));
        }
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
    /**Resolve a list of Package Groups

Sends a `POST` request to `/api/v1/catalog/resolve`

*/
    pub async fn resolve_api_v1_catalog_resolve_post<'a>(
        &'a self,
        body: &'a types::PackageGroups,
    ) -> Result<
        ResponseValue<types::ResolvedPackageGroups>,
        Error<types::HttpValidationError>,
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
