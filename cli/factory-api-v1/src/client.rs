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
    /**Payload posted by the build coordinator to
``POST /api/v1/factory/callbacks/builds/{task_id}``.

Fields match ``build_coordinator/reporter.py:51-56`` exactly.
All four are required by the coordinator wire contract; the
Optional[None] defaults on ``error_message`` and ``exit_code``
accommodate the ``completed`` and ``cancelled`` terminal cases
where neither is meaningful.*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "BuildCallbackPayload",
    ///  "description": "Payload posted by the build coordinator to\n``POST /api/v1/factory/callbacks/builds/{task_id}``.\n\nFields match ``build_coordinator/reporter.py:51-56`` exactly.\nAll four are required by the coordinator wire contract; the\nOptional[None] defaults on ``error_message`` and ``exit_code``\naccommodate the ``completed`` and ``cancelled`` terminal cases\nwhere neither is meaningful.",
    ///  "type": "object",
    ///  "required": [
    ///    "build_id",
    ///    "status"
    ///  ],
    ///  "properties": {
    ///    "build_id": {
    ///      "title": "Build Id",
    ///      "type": "string"
    ///    },
    ///    "error_message": {
    ///      "title": "Error Message",
    ///      "type": [
    ///        "string",
    ///        "null"
    ///      ]
    ///    },
    ///    "exit_code": {
    ///      "title": "Exit Code",
    ///      "type": [
    ///        "integer",
    ///        "null"
    ///      ]
    ///    },
    ///    "status": {
    ///      "title": "Status",
    ///      "type": "string",
    ///      "enum": [
    ///        "queued",
    ///        "dispatching",
    ///        "running",
    ///        "completed",
    ///        "failed",
    ///        "timed_out",
    ///        "cancelled"
    ///      ]
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct BuildCallbackPayload {
        pub build_id: ::std::string::String,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub error_message: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub exit_code: ::std::option::Option<i64>,
        pub status: Status,
    }
    impl ::std::convert::From<&BuildCallbackPayload> for BuildCallbackPayload {
        fn from(value: &BuildCallbackPayload) -> Self {
            value.clone()
        }
    }
    ///Paginated list of builds.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "BuildListResponse",
    ///  "description": "Paginated list of builds.",
    ///  "type": "object",
    ///  "required": [
    ///    "builds",
    ///    "page",
    ///    "page_size",
    ///    "total"
    ///  ],
    ///  "properties": {
    ///    "builds": {
    ///      "title": "Builds",
    ///      "type": "array",
    ///      "items": {
    ///        "$ref": "#/components/schemas/BuildResponse"
    ///      }
    ///    },
    ///    "page": {
    ///      "title": "Page",
    ///      "type": "integer"
    ///    },
    ///    "page_size": {
    ///      "title": "Page Size",
    ///      "type": "integer"
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
    pub struct BuildListResponse {
        pub builds: ::std::vec::Vec<BuildResponse>,
        pub page: i64,
        pub page_size: i64,
        pub total: i64,
    }
    impl ::std::convert::From<&BuildListResponse> for BuildListResponse {
        fn from(value: &BuildListResponse) -> Self {
            value.clone()
        }
    }
    /**Build-specific details with optional task sub-object.

The task field is None for undispatched builds (task_id IS NULL
in factory_builds).

The status field is the build's effective current status, computed
server-side from the freshest authoritative source:

- Pre-dispatch (task_id IS NULL): computed from
  factory_builds.cancelled_at — ``"cancelled"`` when set, else
  ``"pending"``. Neither word is stored; the timestamp is the only
  persisted pre-dispatch state.
- Dispatched: tasks.status (``"running"``, ``"completed"``,
  ``"failed"``, ``"cancelled"``).
- On cancel responses: Build Coordinator's returned status, which
  is fresher than the local row (which lags until BC's callback
  lands), normalized into the task vocabulary above — BC's
  ``timed_out`` is reported as ``failed``, exactly as the callback
  path persists it. The field never carries a word outside the
  union of the two sets above.

Staleness: after handoff the field tracks tasks.status, which only
advances when Build Coordinator's terminal callback lands. When
callbacks are disabled (no callback base URL configured — a
supported worker configuration), the post-handoff status stays at
the last persisted value (typically ``"running"``) indefinitely;
a cancel response is then the only place a fresher
coordinator-reported status appears.*/
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "BuildResponse",
    ///  "description": "Build-specific details with optional task sub-object.\n\nThe task field is None for undispatched builds (task_id IS NULL\nin factory_builds).\n\nThe status field is the build's effective current status, computed\nserver-side from the freshest authoritative source:\n\n- Pre-dispatch (task_id IS NULL): computed from\n  factory_builds.cancelled_at — ``\"cancelled\"`` when set, else\n  ``\"pending\"``. Neither word is stored; the timestamp is the only\n  persisted pre-dispatch state.\n- Dispatched: tasks.status (``\"running\"``, ``\"completed\"``,\n  ``\"failed\"``, ``\"cancelled\"``).\n- On cancel responses: Build Coordinator's returned status, which\n  is fresher than the local row (which lags until BC's callback\n  lands), normalized into the task vocabulary above — BC's\n  ``timed_out`` is reported as ``failed``, exactly as the callback\n  path persists it. The field never carries a word outside the\n  union of the two sets above.\n\nStaleness: after handoff the field tracks tasks.status, which only\nadvances when Build Coordinator's terminal callback lands. When\ncallbacks are disabled (no callback base URL configured — a\nsupported worker configuration), the post-handoff status stays at\nthe last persisted value (typically ``\"running\"``) indefinitely;\na cancel response is then the only place a fresher\ncoordinator-reported status appears.",
    ///  "type": "object",
    ///  "required": [
    ///    "attr_path",
    ///    "build_id",
    ///    "build_type",
    ///    "catalog_name",
    ///    "created_at",
    ///    "nixpkgs_revision",
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
    ///    "build_id": {
    ///      "title": "Build Id",
    ///      "type": "integer"
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
    ///    "source_commit_sha": {
    ///      "title": "Source Commit Sha",
    ///      "type": "string"
    ///    },
    ///    "source_repo_url": {
    ///      "title": "Source Repo Url",
    ///      "type": "string"
    ///    },
    ///    "status": {
    ///      "title": "Status",
    ///      "type": "string"
    ///    },
    ///    "system": {
    ///      "title": "System",
    ///      "type": "string"
    ///    },
    ///    "task": {
    ///      "$ref": "#/components/schemas/TaskResponse"
    ///    }
    ///  }
    ///}
    /// ```
    /// </details>
    #[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
    pub struct BuildResponse {
        pub attr_path: ::std::string::String,
        pub build_id: i64,
        pub build_type: ::std::string::String,
        pub catalog_name: ::std::string::String,
        pub created_at: ::chrono::DateTime<::chrono::offset::Utc>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub exit_code: ::std::option::Option<i64>,
        pub nixpkgs_revision: ::std::string::String,
        pub source_commit_sha: ::std::string::String,
        pub source_repo_url: ::std::string::String,
        pub status: ::std::string::String,
        pub system: ::std::string::String,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub task: ::std::option::Option<TaskResponse>,
    }
    impl ::std::convert::From<&BuildResponse> for BuildResponse {
        fn from(value: &BuildResponse) -> Self {
            value.clone()
        }
    }
    ///JSON body returned for all error responses on the builds endpoints.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "ErrorResponse",
    ///  "description": "JSON body returned for all error responses on the builds endpoints.",
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
    ///`Status`
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "Status",
    ///  "type": "string",
    ///  "enum": [
    ///    "queued",
    ///    "dispatching",
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
    pub enum Status {
        #[serde(rename = "queued")]
        Queued,
        #[serde(rename = "dispatching")]
        Dispatching,
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
    impl ::std::convert::From<&Self> for Status {
        fn from(value: &Status) -> Self {
            value.clone()
        }
    }
    impl ::std::fmt::Display for Status {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
            match *self {
                Self::Queued => f.write_str("queued"),
                Self::Dispatching => f.write_str("dispatching"),
                Self::Running => f.write_str("running"),
                Self::Completed => f.write_str("completed"),
                Self::Failed => f.write_str("failed"),
                Self::TimedOut => f.write_str("timed_out"),
                Self::Cancelled => f.write_str("cancelled"),
            }
        }
    }
    impl ::std::str::FromStr for Status {
        type Err = self::error::ConversionError;
        fn from_str(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            match value {
                "queued" => Ok(Self::Queued),
                "dispatching" => Ok(Self::Dispatching),
                "running" => Ok(Self::Running),
                "completed" => Ok(Self::Completed),
                "failed" => Ok(Self::Failed),
                "timed_out" => Ok(Self::TimedOut),
                "cancelled" => Ok(Self::Cancelled),
                _ => Err("invalid value".into()),
            }
        }
    }
    impl ::std::convert::TryFrom<&str> for Status {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &str,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<&::std::string::String> for Status {
        type Error = self::error::ConversionError;
        fn try_from(
            value: &::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    impl ::std::convert::TryFrom<::std::string::String> for Status {
        type Error = self::error::ConversionError;
        fn try_from(
            value: ::std::string::String,
        ) -> ::std::result::Result<Self, self::error::ConversionError> {
            value.parse()
        }
    }
    ///Generic task lifecycle — same shape for all operation types.
    ///
    /// <details><summary>JSON schema</summary>
    ///
    /// ```json
    ///{
    ///  "title": "TaskResponse",
    ///  "description": "Generic task lifecycle — same shape for all operation types.",
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
    ///      "title": "Error Class",
    ///      "type": [
    ///        "string",
    ///        "null"
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
    ///      "title": "Status",
    ///      "type": "string"
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
        pub error_class: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub error_message: ::std::option::Option<::std::string::String>,
        #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
        pub started_at: ::std::option::Option<::chrono::DateTime<::chrono::offset::Utc>>,
        pub status: ::std::string::String,
        pub task_id: i64,
        pub task_type: ::std::string::String,
        pub updated_at: ::chrono::DateTime<::chrono::offset::Utc>,
    }
    impl ::std::convert::From<&TaskResponse> for TaskResponse {
        fn from(value: &TaskResponse) -> Self {
            value.clone()
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
impl Client {
    /**List Builds

Return a paginated list of builds.

Use the `status` query parameter to filter:
- ``undispatched``: builds with no task (task_id IS NULL)
- Any other value: filters by tasks.status (e.g. ``running``)

Sends a `GET` request to `/api/v1/factory/builds`

*/
    pub async fn list_builds_api_v1_factory_builds_get<'a>(
        &'a self,
        page: Option<u64>,
        page_size: Option<::std::num::NonZeroU64>,
        status: Option<&'a str>,
    ) -> Result<ResponseValue<types::BuildListResponse>, Error<types::ErrorResponse>> {
        let url = format!("{}/api/v1/factory/builds", self.baseurl);
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
            .query(&progenitor_client::QueryParam::new("page_size", &page_size))
            .query(&progenitor_client::QueryParam::new("status", &status))
            .headers(header_map)
            .build()?;
        let info = OperationInfo {
            operation_id: "list_builds_api_v1_factory_builds_get",
        };
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
    /**Get Build

Return a single build by ID with its task sub-object.

Sends a `GET` request to `/api/v1/factory/builds/{build_id}`

*/
    pub async fn get_build_api_v1_factory_builds_build_id_get<'a>(
        &'a self,
        build_id: i64,
    ) -> Result<ResponseValue<types::BuildResponse>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/factory/builds/{}",
            self.baseurl,
            encode_path(&build_id.to_string()),
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
            operation_id: "get_build_api_v1_factory_builds_build_id_get",
        };
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
    /**Cancel Build

Cancel a build.

Handles both active builds (delegated to Build Coordinator) and
pre-dispatch builds (cancelled FS-only via an atomic row lock).

The operation is idempotent with respect to terminal state. When
the build has already reached a terminal state (``cancelled``,
``completed``, or ``failed``) — whether Build Coordinator reports
it or the local task row records it — the response is still 200
and the ``status`` field carries that terminal state. Coordinator
statuses are normalized into the task vocabulary before they are
surfaced (BC's ``timed_out`` is reported as ``failed``, exactly as
the callback path persists it), so the cancel response and a
subsequent ``GET`` always agree.

Cancelling a pre-dispatch build permanently retires its identity
tuple: ``uq_factory_build_identity`` dedup treats the cancelled
row like any other terminal build, so a later event expanding to
the same identity inserts nothing. Recovery is manual by design —
a cancel that ambient event traffic could overturn would not be a
cancel.

Outcomes:
    200 — Build cancelled, or already terminal. ``BuildResponse.status``
          reflects the effective state.
    404 — No build with the given ID.
    502 — Build Coordinator unreachable or returned an unexpected
          error; or the coordinator does not know the build yet
          because its dispatch is in flight (the worker commits
          its claim before the HTTP submit), or no longer knows it
          (coordinator restart or purge). In every 502 case the
          correct client action is retry with backoff.

An audit log line is emitted on every path, including unhandled
exceptions (``outcome=internal_error``).

Sends a `DELETE` request to `/api/v1/factory/builds/{build_id}`

*/
    pub async fn cancel_build_api_v1_factory_builds_build_id_delete<'a>(
        &'a self,
        build_id: i64,
    ) -> Result<ResponseValue<types::BuildResponse>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/factory/builds/{}",
            self.baseurl,
            encode_path(&build_id.to_string()),
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
            operation_id: "cancel_build_api_v1_factory_builds_build_id_delete",
        };
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
            502u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            _ => Err(Error::UnexpectedResponse(response)),
        }
    }
    /**Get Build Logs

Proxy the log body for a build, source-faithfully.

Fetches the log content from Build Coordinator and returns it as
``text/plain; charset=utf-8`` without modification.  No escape
stripping, no control-byte filtering, no timestamp injection.
Terminal-escape handling is the CLI consumer's responsibility.
The only hygiene applied is at the coordinator client layer: valid
UTF-8 output and a size-capped read.  No JSON envelope; the body
*is* the log.

Returns:
    200 with the log body.
    404 if the build does not exist, has no Build Coordinator
        counterpart (task_id IS NULL), or the coordinator itself
        returns 404.
    500 if an unexpected error occurs (e.g. get_build_by_id raises).
    502 if Build Coordinator is unreachable or returns a 5xx error.

Sends a `GET` request to `/api/v1/factory/builds/{build_id}/logs`

*/
    pub async fn get_build_logs_api_v1_factory_builds_build_id_logs_get<'a>(
        &'a self,
        build_id: i64,
    ) -> Result<ResponseValue<ByteStream>, Error<types::ErrorResponse>> {
        let url = format!(
            "{}/api/v1/factory/builds/{}/logs",
            self.baseurl,
            encode_path(&build_id.to_string()),
        );
        let mut header_map = ::reqwest::header::HeaderMap::with_capacity(1usize);
        header_map
            .append(
                ::reqwest::header::HeaderName::from_static("api-version"),
                ::reqwest::header::HeaderValue::from_static(Self::api_version()),
            );
        #[allow(unused_mut)]
        let mut request = self.client.get(url).headers(header_map).build()?;
        let info = OperationInfo {
            operation_id: "get_build_logs_api_v1_factory_builds_build_id_logs_get",
        };
        self.pre(&mut request, &info).await?;
        let result = self.exec(request, &info).await;
        self.post(&result, &info).await?;
        let response = result?;
        match response.status().as_u16() {
            200u16 => Ok(ResponseValue::stream(response)),
            404u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            422u16 => {
                Err(Error::ErrorResponse(ResponseValue::from_response(response).await?))
            }
            502u16 => {
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
