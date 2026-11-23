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
    ///A repository URL and the revision locked from it.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "BuildInputSource",
    ///  "description": "A repository URL and the revision locked from it.",
    ///  "type": "object",
    ///  "required": [
    ///    "rev",
    ///    "url"
    ///  ],
    ///  "properties": {
    ///    "rev": {
    ///      "title": "Rev",
    ///      "description": "The revision locked from it.",
    ///      "type": "string"
    ///    },
    ///    "url": {
    ///      "title": "Url",
    ///      "description": "The repository's URL.",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct BuildInputSource {
        ///The revision locked from it.
        pub rev: ::std::string::String,
        ///The repository's URL.
        pub url: ::std::string::String,
    }
    impl ::std::convert::From<&BuildInputSource> for BuildInputSource {
        fn from(value: &BuildInputSource) -> Self {
            value.clone()
        }
    }
    ///An output of the derivation a run published.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "BuildOutput",
    ///  "description": "An output of the derivation a run published.",
    ///  "type": "object",
    ///  "required": [
    ///    "is_default",
    ///    "name",
    ///    "store_path"
    ///  ],
    ///  "properties": {
    ///    "closure_size": {
    ///      "title": "Closure Size",
    ///      "description": "Total size in bytes of the NARs in the store path's closure. Null when none has been recorded. The base-catalog crawler records none, so a base-catalog store path has one only when a publish recorded it as part of a closure.",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "is_default": {
    ///      "title": "Is Default",
    ///      "description": "Whether the output is installed by default: it is named in the derivation's `outputs_to_install`.",
    ///      "type": "boolean"
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "type": "string"
    ///    },
    ///    "nar_size": {
    ///      "title": "Nar Size",
    ///      "description": "Size in bytes of the store path's NAR. Null when no size has been recorded: the publish has not finished uploading, failed to record it, or went to a store that records no sizes. On a base-catalog store path, also null when the crawler has not recorded the path or cache.nixos.org has no NAR for it.",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
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
    pub struct BuildOutput {
        ///Total size in bytes of the NARs in the store path's closure. Null when none has been recorded. The base-catalog crawler records none, so a base-catalog store path has one only when a publish recorded it as part of a closure.
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub closure_size: ::std::option::Option<i64>,
        ///Whether the output is installed by default: it is named in the derivation's `outputs_to_install`.
        pub is_default: bool,
        pub name: ::std::string::String,
        ///Size in bytes of the store path's NAR. Null when no size has been recorded: the publish has not finished uploading, failed to record it, or went to a store that records no sizes. On a base-catalog store path, also null when the crawler has not recorded the path or cache.nixos.org has no NAR for it.
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub nar_size: ::std::option::Option<i64>,
        pub store_path: ::std::string::String,
    }
    impl ::std::convert::From<&BuildOutput> for BuildOutput {
        fn from(value: &BuildOutput) -> Self {
            value.clone()
        }
    }
    ///A catalog package locked as one of a run's inputs.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "CatalogInput",
    ///  "description": "A catalog package locked as one of a run's inputs.",
    ///  "type": "object",
    ///  "required": [
    ///    "name",
    ///    "rev",
    ///    "url"
    ///  ],
    ///  "properties": {
    ///    "name": {
    ///      "title": "Name",
    ///      "description": "The input's catalog and attribute path, as `<catalog>/<attr_path>`.",
    ///      "type": "string"
    ///    },
    ///    "rev": {
    ///      "title": "Rev",
    ///      "description": "The revision locked from it.",
    ///      "type": "string"
    ///    },
    ///    "url": {
    ///      "title": "Url",
    ///      "description": "The repository's URL.",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct CatalogInput {
        ///The input's catalog and attribute path, as `<catalog>/<attr_path>`.
        pub name: ::std::string::String,
        ///The revision locked from it.
        pub rev: ::std::string::String,
        ///The repository's URL.
        pub url: ::std::string::String,
    }
    impl ::std::convert::From<&CatalogInput> for CatalogInput {
        fn from(value: &CatalogInput) -> Self {
            value.clone()
        }
    }
    ///Window summary for one catalog.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "CatalogRunStats",
    ///  "description": "Window summary for one catalog.",
    ///  "type": "object",
    ///  "required": [
    ///    "catalog",
    ///    "completed_seconds",
    ///    "counts"
    ///  ],
    ///  "properties": {
    ///    "catalog": {
    ///      "title": "Catalog",
    ///      "type": "string"
    ///    },
    ///    "completed_seconds": {
    ///      "title": "Completed Seconds",
    ///      "type": "integer"
    ///    },
    ///    "counts": {
    ///      "$ref": "#/components/schemas/RunStatusCounts"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct CatalogRunStats {
        pub catalog: ::std::string::String,
        pub completed_seconds: i64,
        pub counts: RunStatusCounts,
    }
    impl ::std::convert::From<&CatalogRunStats> for CatalogRunStats {
        fn from(value: &CatalogRunStats) -> Self {
            value.clone()
        }
    }
    /**Effective status of a run, derived server-side and never stored.

A dispatched run's task lifecycle is authoritative, with
timed_out reconstructed from a failed task whose error class is
'timeout'; a run cancelled before dispatch is cancelled. A run
is pending until it starts: undispatched and uncancelled, or
dispatched with a running task the coordinator has not started,
such as one waiting in its queue. These six values are
exactly what the derivation can emit; emittable, filterable, and
the whole vocabulary are the same set. The member order here is
the documentation order.*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "EffectiveRunStatus",
    ///  "description": "Effective status of a run, derived server-side and never stored.\n\nA dispatched run's task lifecycle is authoritative, with\ntimed_out reconstructed from a failed task whose error class is\n'timeout'; a run cancelled before dispatch is cancelled. A run\nis pending until it starts: undispatched and uncancelled, or\ndispatched with a running task the coordinator has not started,\nsuch as one waiting in its queue. These six values are\nexactly what the derivation can emit; emittable, filterable, and\nthe whole vocabulary are the same set. The member order here is\nthe documentation order.",
    ///  "type": "string",
    ///  "enum": [
    ///    "pending",
    ///    "running",
    ///    "completed",
    ///    "failed",
    ///    "timed_out",
    ///    "cancelled"
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
    pub enum EffectiveRunStatus {
        #[serde(rename = "pending")]
        Pending,
        #[serde(rename = "running")]
        Running,
        #[serde(rename = "completed")]
        Completed,
        #[serde(rename = "failed")]
        Failed,
        #[serde(rename = "timed_out")]
        TimedOut,
        #[serde(rename = "cancelled")]
        Cancelled,
    }
    impl ::std::convert::From<&Self> for EffectiveRunStatus {
        fn from(value: &EffectiveRunStatus) -> Self {
            value.clone()
        }
    }
    impl ::std::fmt::Display for EffectiveRunStatus {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
            match *self {
                Self::Pending => f.write_str("pending"),
                Self::Running => f.write_str("running"),
                Self::Completed => f.write_str("completed"),
                Self::Failed => f.write_str("failed"),
                Self::TimedOut => f.write_str("timed_out"),
                Self::Cancelled => f.write_str("cancelled"),
            }
        }
    }
    impl ::std::str::FromStr for EffectiveRunStatus {
        type Err = self::error::ConversionError;
        fn from_str(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            match value {
                "pending" => Ok(Self::Pending),
                "running" => Ok(Self::Running),
                "completed" => Ok(Self::Completed),
                "failed" => Ok(Self::Failed),
                "timed_out" => Ok(Self::TimedOut),
                "cancelled" => Ok(Self::Cancelled),
                _ => Err("invalid value".into()),
            }
        }
    }
    impl ::std::convert::TryFrom<&str> for EffectiveRunStatus {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<&::std::string::String> for EffectiveRunStatus {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<::std::string::String> for EffectiveRunStatus {
        type Error = self::error::ConversionError;
        fn try_from(
            value: ::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    /**JSON body returned for all error responses on the runs and
stats endpoints.*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ErrorResponse",
    ///  "description": "JSON body returned for all error responses on the runs and\nstats endpoints.",
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
    ///The catalog package a run published.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "PublishedPackage",
    ///  "description": "The catalog package a run published.",
    ///  "type": "object",
    ///  "required": [
    ///    "derivation",
    ///    "description",
    ///    "license",
    ///    "name",
    ///    "source_rev_count",
    ///    "version"
    ///  ],
    ///  "properties": {
    ///    "derivation": {
    ///      "title": "Derivation",
    ///      "description": "The published derivation's path, as environment locks record it.",
    ///      "type": "string"
    ///    },
    ///    "description": {
    ///      "title": "Description",
    ///      "description": "The published derivation's description, null when it has none.",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "license": {
    ///      "title": "License",
    ///      "description": "The published derivation's license as the catalog records it, such as `MIT` or `[ MIT, Apache-2.0 ]`; null when it has none.",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "name": {
    ///      "title": "Name",
    ///      "description": "The published derivation's name, such as `hello-2.12.1`.",
    ///      "type": "string"
    ///    },
    ///    "source_rev_count": {
    ///      "title": "Source Rev Count",
    ///      "description": "The revision count of the source page the package was built from. One part of the catalog build's coordinate: builds of the same source revision with different locked inputs or base page share it.",
    ///      "type": "integer"
    ///    },
    ///    "version": {
    ///      "title": "Version",
    ///      "description": "The published derivation's version, null when it has none.",
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
    pub struct PublishedPackage {
        ///The published derivation's path, as environment locks record it.
        pub derivation: ::std::string::String,
        ///The published derivation's description, null when it has none.
        pub description: ::std::option::Option<::std::string::String>,
        ///The published derivation's license as the catalog records it, such as `MIT` or `[ MIT, Apache-2.0 ]`; null when it has none.
        pub license: ::std::option::Option<::std::string::String>,
        ///The published derivation's name, such as `hello-2.12.1`.
        pub name: ::std::string::String,
        ///The revision count of the source page the package was built from. One part of the catalog build's coordinate: builds of the same source revision with different locked inputs or base page share it.
        pub source_rev_count: i64,
        ///The published derivation's version, null when it has none.
        pub version: ::std::option::Option<::std::string::String>,
    }
    impl ::std::convert::From<&PublishedPackage> for PublishedPackage {
        fn from(value: &PublishedPackage) -> Self {
            value.clone()
        }
    }
    ///A single run, with the outputs of the derivation it published.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "RunDetailResponse",
    ///  "description": "A single run, with the outputs of the derivation it published.",
    ///  "type": "object",
    ///  "required": [
    ///    "attr_path",
    ///    "build_type",
    ///    "catalog_name",
    ///    "created_at",
    ///    "nixpkgs_revision",
    ///    "run_id",
    ///    "source_commit_sha",
    ///    "source_repo_url",
    ///    "status",
    ///    "system"
    ///  ],
    ///  "properties": {
    ///    "attr_path": {
    ///      "title": "Attr Path",
    ///      "type": "string"
    ///    },
    ///    "build_type": {
    ///      "title": "Build Type",
    ///      "type": "string"
    ///    },
    ///    "catalog_name": {
    ///      "title": "Catalog Name",
    ///      "type": "string"
    ///    },
    ///    "created_at": {
    ///      "title": "Created At",
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "exit_code": {
    ///      "title": "Exit Code",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "nixpkgs_revision": {
    ///      "title": "Nixpkgs Revision",
    ///      "type": "string"
    ///    },
    ///    "outputs": {
    ///      "title": "Outputs",
    ///      "description": "Every output of the derivation the run published, with its recorded sizes. Empty when no published package is linked to the run.",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/BuildOutput"
    ///      }
    ///    },
    ///    "published_package": {
    ///      "description": "The package the run published, null when no publish is linked.",
    ///      "oneOf": [
    ///        {
    ///          "type": "null"
    ///        },
    ///        {
    ///          "allOf": [
    ///            {
    ///              "$ref": "#/components/schemas/PublishedPackage"
    ///            }
    ///          ]
    ///        }
    ///      ]
    ///    },
    ///    "run_id": {
    ///      "title": "Run Id",
    ///      "type": "integer"
    ///    },
    ///    "source_commit_sha": {
    ///      "title": "Source Commit Sha",
    ///      "type": "string"
    ///    },
    ///    "source_repo_url": {
    ///      "title": "Source Repo Url",
    ///      "type": "string"
    ///    },
    ///    "status": {
    ///      "$ref": "#/components/schemas/EffectiveRunStatus"
    ///    },
    ///    "system": {
    ///      "title": "System",
    ///      "type": "string"
    ///    },
    ///    "task": {
    ///      "oneOf": [
    ///        {
    ///          "type": "null"
    ///        },
    ///        {
    ///          "allOf": [
    ///            {
    ///              "$ref": "#/components/schemas/TaskResponse"
    ///            }
    ///          ]
    ///        }
    ///      ]
    ///    },
    ///    "trigger": {
    ///      "$ref": "#/components/schemas/RunTrigger"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct RunDetailResponse {
        pub attr_path: ::std::string::String,
        pub build_type: ::std::string::String,
        pub catalog_name: ::std::string::String,
        pub created_at: ::chrono::DateTime<::chrono::offset::Utc>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub exit_code: ::std::option::Option<i64>,
        pub nixpkgs_revision: ::std::string::String,
        ///Every output of the derivation the run published, with its recorded sizes. Empty when no published package is linked to the run.
        #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
        pub outputs: ::std::vec::Vec<BuildOutput>,
        ///The package the run published, null when no publish is linked.
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub published_package: ::std::option::Option<PublishedPackage>,
        pub run_id: i64,
        pub source_commit_sha: ::std::string::String,
        pub source_repo_url: ::std::string::String,
        pub status: EffectiveRunStatus,
        pub system: ::std::string::String,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub task: ::std::option::Option<TaskResponse>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub trigger: ::std::option::Option<RunTrigger>,
    }
    impl ::std::convert::From<&RunDetailResponse> for RunDetailResponse {
        fn from(value: &RunDetailResponse) -> Self {
            value.clone()
        }
    }
    /**Package inputs associated with a run.

The run's source is on ``RunResponse``.*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "RunInputsResponse",
    ///  "description": "Package inputs associated with a run.\n\nThe run's source is on ``RunResponse``.",
    ///  "type": "object",
    ///  "required": [
    ///    "base_catalog",
    ///    "catalog_inputs"
    ///  ],
    ///  "properties": {
    ///    "base_catalog": {
    ///      "description": "The base catalog page recorded when the run published its package. For a NEF run with no publish linked, the base catalog revision it was dispatched with. Null when neither is recorded.",
    ///      "oneOf": [
    ///        {
    ///          "type": "null"
    ///        },
    ///        {
    ///          "allOf": [
    ///            {
    ///              "$ref": "#/components/schemas/BuildInputSource"
    ///            }
    ///          ]
    ///        }
    ///      ]
    ///    },
    ///    "catalog_inputs": {
    ///      "title": "Catalog Inputs",
    ///      "description": "Every catalog package locked when the run published its package, directly or through another input, at the revision recorded then, sorted by name, then revision. Empty when none was locked; null when no publish is linked.",
    ///      "type": [
    ///        "array",
    ///        "null"
    ///      ],
    ///      "items": {
    ///        "$ref": "#/components/schemas/CatalogInput"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct RunInputsResponse {
        ///The base catalog page recorded when the run published its package. For a NEF run with no publish linked, the base catalog revision it was dispatched with. Null when neither is recorded.
        pub base_catalog: ::std::option::Option<BuildInputSource>,
        ///Every catalog package locked when the run published its package, directly or through another input, at the revision recorded then, sorted by name, then revision. Empty when none was locked; null when no publish is linked.
        pub catalog_inputs: ::std::option::Option<::std::vec::Vec<CatalogInput>>,
    }
    impl ::std::convert::From<&RunInputsResponse> for RunInputsResponse {
        fn from(value: &RunInputsResponse) -> Self {
            value.clone()
        }
    }
    ///Paginated list of runs.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "RunListResponse",
    ///  "description": "Paginated list of runs.",
    ///  "type": "object",
    ///  "required": [
    ///    "page",
    ///    "page_size",
    ///    "runs",
    ///    "total"
    ///  ],
    ///  "properties": {
    ///    "page": {
    ///      "title": "Page",
    ///      "type": "integer"
    ///    },
    ///    "page_size": {
    ///      "title": "Page Size",
    ///      "type": "integer"
    ///    },
    ///    "runs": {
    ///      "title": "Runs",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/RunResponse"
    ///      }
    ///    },
    ///    "total": {
    ///      "title": "Total",
    ///      "type": "integer"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct RunListResponse {
        pub page: i64,
        pub page_size: i64,
        pub runs: ::std::vec::Vec<RunResponse>,
        pub total: i64,
    }
    impl ::std::convert::From<&RunListResponse> for RunListResponse {
        fn from(value: &RunListResponse) -> Self {
            value.clone()
        }
    }
    /**Run-specific details with optional task sub-object.

The task field is None for undispatched runs (task_id IS NULL
in factory_runs).

The status field is the run's effective current status — the
EffectiveRunStatus vocabulary — computed server-side from the
freshest authoritative source:

- Pre-dispatch (task_id IS NULL): computed from
  factory_runs.cancelled_at — ``"cancelled"`` when set, else
  ``"pending"``. Neither word is stored; the timestamp is the only
  persisted pre-dispatch state.
- Dispatched: tasks.status (``"running"``, ``"completed"``,
  ``"failed"``, ``"cancelled"``), with ``"timed_out"``
  reconstructed from the persisted footprint of an execution
  timeout (status='failed' + error_class='timeout'). A running
  task the coordinator has not yet started (no started_at), such
  as one waiting in its queue, reads as ``"pending"``. A
  submit-time ``dispatch_timeout`` is a different failure — the
  run never observably started — and reads as ``"failed"``.
- On cancel responses: Build Coordinator's returned status, which
  is fresher than the local row (which lags until BC's callback
  lands), put through the same derivation — BC's ``timed_out``
  surfaces as ``"timed_out"``, agreeing with what a subsequent GET
  reconstructs once the callback persists the footprint. The field
  never carries a word outside the effective vocabulary.

Staleness: after handoff the field tracks the task, which advances
when Build Coordinator's start and terminal callbacks land, or when
the reconciler reads a status whose callback never came. Between
the two, a cancel response is the only place a fresher
coordinator-reported status appears.*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "RunResponse",
    ///  "description": "Run-specific details with optional task sub-object.\n\nThe task field is None for undispatched runs (task_id IS NULL\nin factory_runs).\n\nThe status field is the run's effective current status — the\nEffectiveRunStatus vocabulary — computed server-side from the\nfreshest authoritative source:\n\n- Pre-dispatch (task_id IS NULL): computed from\n  factory_runs.cancelled_at — ``\"cancelled\"`` when set, else\n  ``\"pending\"``. Neither word is stored; the timestamp is the only\n  persisted pre-dispatch state.\n- Dispatched: tasks.status (``\"running\"``, ``\"completed\"``,\n  ``\"failed\"``, ``\"cancelled\"``), with ``\"timed_out\"``\n  reconstructed from the persisted footprint of an execution\n  timeout (status='failed' + error_class='timeout'). A running\n  task the coordinator has not yet started (no started_at), such\n  as one waiting in its queue, reads as ``\"pending\"``. A\n  submit-time ``dispatch_timeout`` is a different failure — the\n  run never observably started — and reads as ``\"failed\"``.\n- On cancel responses: Build Coordinator's returned status, which\n  is fresher than the local row (which lags until BC's callback\n  lands), put through the same derivation — BC's ``timed_out``\n  surfaces as ``\"timed_out\"``, agreeing with what a subsequent GET\n  reconstructs once the callback persists the footprint. The field\n  never carries a word outside the effective vocabulary.\n\nStaleness: after handoff the field tracks the task, which advances\nwhen Build Coordinator's start and terminal callbacks land, or when\nthe reconciler reads a status whose callback never came. Between\nthe two, a cancel response is the only place a fresher\ncoordinator-reported status appears.",
    ///  "type": "object",
    ///  "required": [
    ///    "attr_path",
    ///    "build_type",
    ///    "catalog_name",
    ///    "created_at",
    ///    "nixpkgs_revision",
    ///    "run_id",
    ///    "source_commit_sha",
    ///    "source_repo_url",
    ///    "status",
    ///    "system"
    ///  ],
    ///  "properties": {
    ///    "attr_path": {
    ///      "title": "Attr Path",
    ///      "type": "string"
    ///    },
    ///    "build_type": {
    ///      "title": "Build Type",
    ///      "type": "string"
    ///    },
    ///    "catalog_name": {
    ///      "title": "Catalog Name",
    ///      "type": "string"
    ///    },
    ///    "created_at": {
    ///      "title": "Created At",
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "exit_code": {
    ///      "title": "Exit Code",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "nixpkgs_revision": {
    ///      "title": "Nixpkgs Revision",
    ///      "type": "string"
    ///    },
    ///    "published_package": {
    ///      "description": "The package the run published, null when no publish is linked.",
    ///      "oneOf": [
    ///        {
    ///          "type": "null"
    ///        },
    ///        {
    ///          "allOf": [
    ///            {
    ///              "$ref": "#/components/schemas/PublishedPackage"
    ///            }
    ///          ]
    ///        }
    ///      ]
    ///    },
    ///    "run_id": {
    ///      "title": "Run Id",
    ///      "type": "integer"
    ///    },
    ///    "source_commit_sha": {
    ///      "title": "Source Commit Sha",
    ///      "type": "string"
    ///    },
    ///    "source_repo_url": {
    ///      "title": "Source Repo Url",
    ///      "type": "string"
    ///    },
    ///    "status": {
    ///      "$ref": "#/components/schemas/EffectiveRunStatus"
    ///    },
    ///    "system": {
    ///      "title": "System",
    ///      "type": "string"
    ///    },
    ///    "task": {
    ///      "oneOf": [
    ///        {
    ///          "type": "null"
    ///        },
    ///        {
    ///          "allOf": [
    ///            {
    ///              "$ref": "#/components/schemas/TaskResponse"
    ///            }
    ///          ]
    ///        }
    ///      ]
    ///    },
    ///    "trigger": {
    ///      "$ref": "#/components/schemas/RunTrigger"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct RunResponse {
        pub attr_path: ::std::string::String,
        pub build_type: ::std::string::String,
        pub catalog_name: ::std::string::String,
        pub created_at: ::chrono::DateTime<::chrono::offset::Utc>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub exit_code: ::std::option::Option<i64>,
        pub nixpkgs_revision: ::std::string::String,
        ///The package the run published, null when no publish is linked.
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub published_package: ::std::option::Option<PublishedPackage>,
        pub run_id: i64,
        pub source_commit_sha: ::std::string::String,
        pub source_repo_url: ::std::string::String,
        pub status: EffectiveRunStatus,
        pub system: ::std::string::String,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub task: ::std::option::Option<TaskResponse>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub trigger: ::std::option::Option<RunTrigger>,
    }
    impl ::std::convert::From<&RunResponse> for RunResponse {
        fn from(value: &RunResponse) -> Self {
            value.clone()
        }
    }
    /**The runs created in a window, summarized per system and per
catalog.

``systems`` lists every system the catalog knows, sorted by name and
zero-padded. ``catalogs`` lists only the catalogs with at least one
run in the window, sorted by name. The two lists partition the
same runs, so their per-status sums agree.*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "RunStatsResponse",
    ///  "description": "The runs created in a window, summarized per system and per\ncatalog.\n\n``systems`` lists every system the catalog knows, sorted by name and\nzero-padded. ``catalogs`` lists only the catalogs with at least one\nrun in the window, sorted by name. The two lists partition the\nsame runs, so their per-status sums agree.",
    ///  "type": "object",
    ///  "required": [
    ///    "catalogs",
    ///    "systems"
    ///  ],
    ///  "properties": {
    ///    "catalogs": {
    ///      "title": "Catalogs",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/CatalogRunStats"
    ///      }
    ///    },
    ///    "systems": {
    ///      "title": "Systems",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/SystemRunStats"
    ///      }
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct RunStatsResponse {
        pub catalogs: ::std::vec::Vec<CatalogRunStats>,
        pub systems: ::std::vec::Vec<SystemRunStats>,
    }
    impl ::std::convert::From<&RunStatsResponse> for RunStatsResponse {
        fn from(value: &RunStatsResponse) -> Self {
            value.clone()
        }
    }
    /**How many runs sit in each effective status.

The fields are named exactly as the run feed's ``status`` filter
values, so each count links into the feed under its own key.*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "RunStatusCounts",
    ///  "description": "How many runs sit in each effective status.\n\nThe fields are named exactly as the run feed's ``status`` filter\nvalues, so each count links into the feed under its own key.",
    ///  "type": "object",
    ///  "required": [
    ///    "cancelled",
    ///    "completed",
    ///    "failed",
    ///    "pending",
    ///    "running",
    ///    "timed_out"
    ///  ],
    ///  "properties": {
    ///    "cancelled": {
    ///      "title": "Cancelled",
    ///      "type": "integer"
    ///    },
    ///    "completed": {
    ///      "title": "Completed",
    ///      "type": "integer"
    ///    },
    ///    "failed": {
    ///      "title": "Failed",
    ///      "type": "integer"
    ///    },
    ///    "pending": {
    ///      "title": "Pending",
    ///      "type": "integer"
    ///    },
    ///    "running": {
    ///      "title": "Running",
    ///      "type": "integer"
    ///    },
    ///    "timed_out": {
    ///      "title": "Timed Out",
    ///      "type": "integer"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct RunStatusCounts {
        pub cancelled: i64,
        pub completed: i64,
        pub failed: i64,
        pub pending: i64,
        pub running: i64,
        pub timed_out: i64,
    }
    impl ::std::convert::From<&RunStatusCounts> for RunStatusCounts {
        fn from(value: &RunStatusCounts) -> Self {
            value.clone()
        }
    }
    ///What scheduled a run.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "RunTrigger",
    ///  "description": "What scheduled a run.",
    ///  "type": "object",
    ///  "properties": {
    ///    "kind": {
    ///      "description": "What scheduled the run: `source_push`, a push to the package's source repository; `base_catalog_push`, a push to the base catalog; `cascade`, a publish of one of the package's dependencies; `fill_in`, a publish of the package itself on another system. Other values may be added. Null when the Factory cannot tell.",
    ///      "oneOf": [
    ///        {
    ///          "type": "null"
    ///        },
    ///        {
    ///          "allOf": [
    ///            {
    ///              "$ref": "#/components/schemas/RunTriggerKind"
    ///            }
    ///          ]
    ///        }
    ///      ]
    ///    },
    ///    "stabilities": {
    ///      "title": "Stabilities",
    ///      "description": "The stabilities that named this run's nixpkgs revision when the run was scheduled. Empty when none did, as for a manifest run, which does not build against the revision. Not the revision's current stabilities, which can move after scheduling.",
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
    pub struct RunTrigger {
        ///What scheduled the run: `source_push`, a push to the package's source repository; `base_catalog_push`, a push to the base catalog; `cascade`, a publish of one of the package's dependencies; `fill_in`, a publish of the package itself on another system. Other values may be added. Null when the Factory cannot tell.
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub kind: ::std::option::Option<RunTriggerKind>,
        ///The stabilities that named this run's nixpkgs revision when the run was scheduled. Empty when none did, as for a manifest run, which does not build against the revision. Not the revision's current stabilities, which can move after scheduling.
        #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
        pub stabilities: ::std::vec::Vec<::std::string::String>,
    }
    impl ::std::convert::From<&RunTrigger> for RunTrigger {
        fn from(value: &RunTrigger) -> Self {
            value.clone()
        }
    }
    impl ::std::default::Default for RunTrigger {
        fn default() -> Self {
            Self {
                kind: Default::default(),
                stabilities: Default::default(),
            }
        }
    }
    ///`RunTriggerKind`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "RunTriggerKind",
    ///  "type": "string"
    ///}
    /// ```
    /// </details>
    #[derive(
        ::serde::Deserialize,
        ::serde::Serialize,
        Clone,
        Debug,
        Eq,
        Hash,
        Ord,
        PartialEq,
        PartialOrd
    )]
    #[serde(transparent)]
    pub struct RunTriggerKind(pub ::std::string::String);
    impl ::std::ops::Deref for RunTriggerKind {
        type Target = ::std::string::String;
        fn deref(&self) -> &::std::string::String {
            &self.0
        }
    }
    impl ::std::convert::From<RunTriggerKind> for ::std::string::String {
        fn from(value: RunTriggerKind) -> Self {
            value.0
        }
    }
    impl ::std::convert::From<&RunTriggerKind> for RunTriggerKind {
        fn from(value: &RunTriggerKind) -> Self {
            value.clone()
        }
    }
    impl ::std::convert::From<::std::string::String> for RunTriggerKind {
        fn from(value: ::std::string::String) -> Self {
            Self(value)
        }
    }
    impl ::std::str::FromStr for RunTriggerKind {
        type Err = ::std::convert::Infallible;
        fn from_str(value: &str) -> ::std::result::Result<Self, Self::Err> {
            Ok(Self(value.to_string()))
        }
    }
    impl ::std::fmt::Display for RunTriggerKind {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
            self.0.fmt(f)
        }
    }
    ///The runtime closure of a run's output store paths.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "RuntimeDependenciesResponse",
    ///  "description": "The runtime closure of a run's output store paths.",
    ///  "type": "object",
    ///  "required": [
    ///    "nodes",
    ///    "truncated"
    ///  ],
    ///  "properties": {
    ///    "nodes": {
    ///      "title": "Nodes",
    ///      "description": "Every store path of the closure, keyed by store path. Empty when the run has no recorded outputs.",
    ///      "type": "object",
    ///      "additionalProperties": {
    ///        "$ref": "#/components/schemas/RuntimeDependencyNode"
    ///      }
    ///    },
    ///    "truncated": {
    ///      "title": "Truncated",
    ///      "description": "True when the walk hit the node cap, so some references are not keys in `nodes`.",
    ///      "type": "boolean"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct RuntimeDependenciesResponse {
        ///Every store path of the closure, keyed by store path. Empty when the run has no recorded outputs.
        pub nodes: ::std::collections::HashMap<
            ::std::string::String,
            RuntimeDependencyNode,
        >,
        ///True when the walk hit the node cap, so some references are not keys in `nodes`.
        pub truncated: bool,
    }
    impl ::std::convert::From<&RuntimeDependenciesResponse>
    for RuntimeDependenciesResponse {
        fn from(value: &RuntimeDependenciesResponse) -> Self {
            value.clone()
        }
    }
    /**One store path of a run's runtime closure.

Both fields are required and nullable, so a generated client types them
as `T | null` and an absent key cannot be mistaken for a recorded empty
value.*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "RuntimeDependencyNode",
    ///  "description": "One store path of a run's runtime closure.\n\nBoth fields are required and nullable, so a generated client types them\nas `T | null` and an absent key cannot be mistaken for a recorded empty\nvalue.",
    ///  "type": "object",
    ///  "required": [
    ///    "nar_size",
    ///    "references"
    ///  ],
    ///  "properties": {
    ///    "nar_size": {
    ///      "title": "Nar Size",
    ///      "description": "NAR size in bytes, or null when no size is recorded.",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "references": {
    ///      "title": "References",
    ///      "description": "Store paths this path references, sorted, without itself. An empty list means it references nothing. Null means no references are recorded for it.",
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
    pub struct RuntimeDependencyNode {
        ///NAR size in bytes, or null when no size is recorded.
        pub nar_size: ::std::option::Option<i64>,
        ///Store paths this path references, sorted, without itself. An empty list means it references nothing. Null means no references are recorded for it.
        pub references: ::std::option::Option<::std::vec::Vec<::std::string::String>>,
    }
    impl ::std::convert::From<&RuntimeDependencyNode> for RuntimeDependencyNode {
        fn from(value: &RuntimeDependencyNode) -> Self {
            value.clone()
        }
    }
    ///Window summary for one system.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "SystemRunStats",
    ///  "description": "Window summary for one system.",
    ///  "type": "object",
    ///  "required": [
    ///    "completed_seconds",
    ///    "counts",
    ///    "system"
    ///  ],
    ///  "properties": {
    ///    "completed_seconds": {
    ///      "title": "Completed Seconds",
    ///      "type": "integer"
    ///    },
    ///    "counts": {
    ///      "$ref": "#/components/schemas/RunStatusCounts"
    ///    },
    ///    "system": {
    ///      "title": "System",
    ///      "type": "string"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct SystemRunStats {
        pub completed_seconds: i64,
        pub counts: RunStatusCounts,
        pub system: ::std::string::String,
    }
    impl ::std::convert::From<&SystemRunStats> for SystemRunStats {
        fn from(value: &SystemRunStats) -> Self {
            value.clone()
        }
    }
    /**Diagnostic class of a failed task.

Mirrors the ``ck_task_error_class`` CHECK as of migration 1.8.2:
error_class IN ('transient', 'permanent', 'timeout',
'dispatch_timeout'); the column is NULL for non-failure
terminals.

- TRANSIENT: says nothing of the run itself, such as a run
  the coordinator lost or gave no status for.
- PERMANENT: will not improve on retry.
- TIMEOUT: the coordinator reported an execution timeout — a
  run that ran and overran its limit.
- DISPATCH_TIMEOUT: the submit HTTP call to the coordinator
  itself timed out, so the run's handoff never observably
  started.

The last two are kept distinct so the read surface shows an
execution timeout as timed_out while a submit timeout reads as
failed.*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "TaskErrorClass",
    ///  "description": "Diagnostic class of a failed task.\n\nMirrors the ``ck_task_error_class`` CHECK as of migration 1.8.2:\nerror_class IN ('transient', 'permanent', 'timeout',\n'dispatch_timeout'); the column is NULL for non-failure\nterminals.\n\n- TRANSIENT: says nothing of the run itself, such as a run\n  the coordinator lost or gave no status for.\n- PERMANENT: will not improve on retry.\n- TIMEOUT: the coordinator reported an execution timeout — a\n  run that ran and overran its limit.\n- DISPATCH_TIMEOUT: the submit HTTP call to the coordinator\n  itself timed out, so the run's handoff never observably\n  started.\n\nThe last two are kept distinct so the read surface shows an\nexecution timeout as timed_out while a submit timeout reads as\nfailed.",
    ///  "type": "string",
    ///  "enum": [
    ///    "transient",
    ///    "permanent",
    ///    "timeout",
    ///    "dispatch_timeout"
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
    pub enum TaskErrorClass {
        #[serde(rename = "transient")]
        Transient,
        #[serde(rename = "permanent")]
        Permanent,
        #[serde(rename = "timeout")]
        Timeout,
        #[serde(rename = "dispatch_timeout")]
        DispatchTimeout,
    }
    impl ::std::convert::From<&Self> for TaskErrorClass {
        fn from(value: &TaskErrorClass) -> Self {
            value.clone()
        }
    }
    impl ::std::fmt::Display for TaskErrorClass {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
            match *self {
                Self::Transient => f.write_str("transient"),
                Self::Permanent => f.write_str("permanent"),
                Self::Timeout => f.write_str("timeout"),
                Self::DispatchTimeout => f.write_str("dispatch_timeout"),
            }
        }
    }
    impl ::std::str::FromStr for TaskErrorClass {
        type Err = self::error::ConversionError;
        fn from_str(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            match value {
                "transient" => Ok(Self::Transient),
                "permanent" => Ok(Self::Permanent),
                "timeout" => Ok(Self::Timeout),
                "dispatch_timeout" => Ok(Self::DispatchTimeout),
                _ => Err("invalid value".into()),
            }
        }
    }
    impl ::std::convert::TryFrom<&str> for TaskErrorClass {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<&::std::string::String> for TaskErrorClass {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<::std::string::String> for TaskErrorClass {
        type Error = self::error::ConversionError;
        fn try_from(
            value: ::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    /**Generic task lifecycle — same shape for all operation types.

The status field carries the task vocabulary — the persisted
lifecycle words, distinct from the derived effective vocabulary
on RunResponse.status. The task sub-object reports the stored
footprint the top-level status is derived from: a timed-out
run's task still reads status='failed' + error_class='timeout'
while the run-level status reads 'timed_out', and a run waiting
in the coordinator's queue has a 'running' task with no
``started_at`` while the run-level status reads 'pending'.*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "TaskResponse",
    ///  "description": "Generic task lifecycle — same shape for all operation types.\n\nThe status field carries the task vocabulary — the persisted\nlifecycle words, distinct from the derived effective vocabulary\non RunResponse.status. The task sub-object reports the stored\nfootprint the top-level status is derived from: a timed-out\nrun's task still reads status='failed' + error_class='timeout'\nwhile the run-level status reads 'timed_out', and a run waiting\nin the coordinator's queue has a 'running' task with no\n``started_at`` while the run-level status reads 'pending'.",
    ///  "type": "object",
    ///  "required": [
    ///    "created_at",
    ///    "status",
    ///    "task_id",
    ///    "task_type",
    ///    "updated_at"
    ///  ],
    ///  "properties": {
    ///    "completed_at": {
    ///      "title": "Completed At",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ],
    ///      "format": "date-time"
    ///    },
    ///    "created_at": {
    ///      "title": "Created At",
    ///      "type": "string",
    ///      "format": "date-time"
    ///    },
    ///    "error_class": {
    ///      "oneOf": [
    ///        {
    ///          "type": "null"
    ///        },
    ///        {
    ///          "allOf": [
    ///            {
    ///              "$ref": "#/components/schemas/TaskErrorClass"
    ///            }
    ///          ]
    ///        }
    ///      ]
    ///    },
    ///    "error_message": {
    ///      "title": "Error Message",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "started_at": {
    ///      "title": "Started At",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ],
    ///      "format": "date-time"
    ///    },
    ///    "status": {
    ///      "$ref": "#/components/schemas/TaskStatus"
    ///    },
    ///    "task_id": {
    ///      "title": "Task Id",
    ///      "type": "integer"
    ///    },
    ///    "task_type": {
    ///      "title": "Task Type",
    ///      "type": "string"
    ///    },
    ///    "updated_at": {
    ///      "title": "Updated At",
    ///      "type": "string",
    ///      "format": "date-time"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct TaskResponse {
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub completed_at: ::std::option::Option<
            ::chrono::DateTime<::chrono::offset::Utc>,
        >,
        pub created_at: ::chrono::DateTime<::chrono::offset::Utc>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub error_class: ::std::option::Option<TaskErrorClass>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub error_message: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub started_at: ::std::option::Option<::chrono::DateTime<::chrono::offset::Utc>>,
        pub status: TaskStatus,
        pub task_id: i64,
        pub task_type: ::std::string::String,
        pub updated_at: ::chrono::DateTime<::chrono::offset::Utc>,
    }
    impl ::std::convert::From<&TaskResponse> for TaskResponse {
        fn from(value: &TaskResponse) -> Self {
            value.clone()
        }
    }
    /**Valid status values for tasks.

Mirrors the CHECK constraint as of migration 1.8.0:
status IN ('running', 'completed', 'failed', 'cancelled').

The claim CTE inserts tasks directly as 'running', and the task
stays 'running' while the coordinator queues the run; its
``started_at`` records when the run actually started. The three
terminal values are written by ``process_callback`` and
``mark_task_failed``.*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "TaskStatus",
    ///  "description": "Valid status values for tasks.\n\nMirrors the CHECK constraint as of migration 1.8.0:\nstatus IN ('running', 'completed', 'failed', 'cancelled').\n\nThe claim CTE inserts tasks directly as 'running', and the task\nstays 'running' while the coordinator queues the run; its\n``started_at`` records when the run actually started. The three\nterminal values are written by ``process_callback`` and\n``mark_task_failed``.",
    ///  "type": "string",
    ///  "enum": [
    ///    "running",
    ///    "completed",
    ///    "failed",
    ///    "cancelled"
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
    pub enum TaskStatus {
        #[serde(rename = "running")]
        Running,
        #[serde(rename = "completed")]
        Completed,
        #[serde(rename = "failed")]
        Failed,
        #[serde(rename = "cancelled")]
        Cancelled,
    }
    impl ::std::convert::From<&Self> for TaskStatus {
        fn from(value: &TaskStatus) -> Self {
            value.clone()
        }
    }
    impl ::std::fmt::Display for TaskStatus {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
            match *self {
                Self::Running => f.write_str("running"),
                Self::Completed => f.write_str("completed"),
                Self::Failed => f.write_str("failed"),
                Self::Cancelled => f.write_str("cancelled"),
            }
        }
    }
    impl ::std::str::FromStr for TaskStatus {
        type Err = self::error::ConversionError;
        fn from_str(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            match value {
                "running" => Ok(Self::Running),
                "completed" => Ok(Self::Completed),
                "failed" => Ok(Self::Failed),
                "cancelled" => Ok(Self::Cancelled),
                _ => Err("invalid value".into()),
            }
        }
    }
    impl ::std::convert::TryFrom<&str> for TaskStatus {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<&::std::string::String> for TaskStatus {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<::std::string::String> for TaskStatus {
        type Error = self::error::ConversionError;
        fn try_from(
            value: ::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
}
#[derive(Clone, Debug)]
/**Client for Flox Factory Service

Flox Factory Service API

Version: unknown*/
pub struct Client {
    pub(crate) baseurl: String,
    pub(crate) client: reqwest::Client,
    pub(crate) inner: crate::hooks::RequestHooks,
}
impl Client {
    /// Create a new client.
    ///
    /// `baseurl` is the base URL provided to the internal
    /// `reqwest::Client`, and should include a scheme and hostname,
    /// as well as port and a path stem if applicable.
    pub fn new(baseurl: &str, inner: crate::hooks::RequestHooks) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let client = {
            let dur = ::std::time::Duration::from_secs(15u64);
            reqwest::ClientBuilder::new().connect_timeout(dur).timeout(dur)
        };
        #[cfg(target_arch = "wasm32")]
        let client = reqwest::ClientBuilder::new();
        Self::new_with_client(baseurl, client.build().unwrap(), inner)
    }
    /// Construct a new client with an existing `reqwest::Client`,
    /// allowing more control over its configuration.
    ///
    /// `baseurl` is the base URL provided to the internal
    /// `reqwest::Client`, and should include a scheme and hostname,
    /// as well as port and a path stem if applicable.
    pub fn new_with_client(
        baseurl: &str,
        client: reqwest::Client,
        inner: crate::hooks::RequestHooks,
    ) -> Self {
        Self {
            baseurl: baseurl.to_string(),
            client,
            inner,
        }
    }
}
impl ClientInfo<crate::hooks::RequestHooks> for Client {
    fn api_version() -> &'static str {
        "unknown"
    }
    fn baseurl(&self) -> &str {
        self.baseurl.as_str()
    }
    fn client(&self) -> &reqwest::Client {
        &self.client
    }
    fn inner(&self) -> &crate::hooks::RequestHooks {
        &self.inner
    }
}
impl ClientHooks<crate::hooks::RequestHooks> for &Client {}
#[allow(clippy::all)]
impl Client {}
/// Items consumers will typically use such as the Client.
pub mod prelude {
    #[allow(unused_imports)]
    pub use super::Client;
}
