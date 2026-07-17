//! Manifest-declared network policy for the activation sandbox.
//!
//! These types back the `[[options.sandbox.network]]` manifest table. Rules
//! are compiled by policy-capable backends (currently `openshell`) into the
//! backend's native policy format and applied when the sandbox is created —
//! deny-by-default networking stays in force for anything not granted here.

use std::fmt::Display;

use serde::{Deserialize, Serialize};

/// A single network grant for the activation sandbox.
///
/// ```toml
/// [[options.sandbox.network]]
/// endpoint = "api.github.com:443"
/// access   = "read-only"                  # default: "full"
/// protocol = "rest"                       # default: "rest"
/// binary   = "curl"                       # default: any process
/// ```
///
/// `binary` scopes the grant to one executable and accepts three forms:
/// an install id from `[install]` (`"curl"` → the locked package's
/// `bin/curl`), `"<install-id>/<exe>"` for packages whose executable name
/// differs from the install id (`"claude-code/.claude-wrapped"`), or an
/// absolute path used verbatim. Install ids resolve to the exact Nix store
/// path locked for the sandbox's system, so grants follow upgrades without
/// editing the rule.
#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq, schemars::JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct SandboxNetworkRule {
    /// The endpoint to grant, as `<HOST>:<PORT>` (e.g. `api.github.com:443`).
    pub endpoint: String,
    /// HTTP access level for the grant. Defaults to `full`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<SandboxNetworkAccess>,
    /// Application protocol the backend's L7 proxy should enforce.
    /// Defaults to `rest`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<SandboxNetworkProtocol>,
    /// Executable the grant is scoped to (see the type-level docs for the
    /// accepted forms). When omitted the grant applies to every process in
    /// the sandbox.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
}

/// HTTP access level of a [`SandboxNetworkRule`].
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, Hash, PartialEq, Eq, Default, schemars::JsonSchema,
)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
pub enum SandboxNetworkAccess {
    /// Read methods only (GET/HEAD/OPTIONS).
    ReadOnly,
    /// Read and write methods, excluding destructive ones.
    ReadWrite,
    /// All methods.
    #[default]
    Full,
}

impl SandboxNetworkAccess {
    /// The kebab-case name used in serialized policies.
    pub fn as_str(&self) -> &'static str {
        match self {
            SandboxNetworkAccess::ReadOnly => "read-only",
            SandboxNetworkAccess::ReadWrite => "read-write",
            SandboxNetworkAccess::Full => "full",
        }
    }
}

impl Display for SandboxNetworkAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Application protocol of a [`SandboxNetworkRule`].
///
/// These are the protocols OpenShell's L7 engine can enforce; `sql` exists
/// in OpenShell's CLI grammar but is rejected under enforcement and is
/// deliberately not offered here.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, Hash, PartialEq, Eq, Default, schemars::JsonSchema,
)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
pub enum SandboxNetworkProtocol {
    /// Plain HTTP request/response APIs.
    #[default]
    Rest,
    /// WebSocket connections.
    Websocket,
    /// GraphQL over HTTP.
    Graphql,
    /// Model Context Protocol.
    Mcp,
    /// JSON-RPC over HTTP.
    JsonRpc,
}

impl SandboxNetworkProtocol {
    /// The kebab-case name used in serialized policies.
    pub fn as_str(&self) -> &'static str {
        match self {
            SandboxNetworkProtocol::Rest => "rest",
            SandboxNetworkProtocol::Websocket => "websocket",
            SandboxNetworkProtocol::Graphql => "graphql",
            SandboxNetworkProtocol::Mcp => "mcp",
            SandboxNetworkProtocol::JsonRpc => "json-rpc",
        }
    }
}

impl Display for SandboxNetworkProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
