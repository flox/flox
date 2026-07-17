//! Batched lockless lookup engine.
//!
//! Turns a flat list of scanned catalog references into a single
//! `/build-inputs/lookup` request, then maps the response into a [BuildLock].
//!
//! The CLI only ever locks one logical set at a time — either a single NEF
//! package's references, or the union of the NEF dependencies of a manifest
//! build — so the public surface takes a plain list of references. The wire
//! protocol's per-group keying is an internal detail (one synthetic group).

use std::collections::BTreeSet;

use floxhub_client::{
    BuildInputsLookupRequest,
    BuildInputsLookupResponse,
    CatalogClientTrait,
    FloxhubClientError,
    LookupGroup,
    ReferencesItem,
    Stability,
    UnresolvableEntry,
};
use tracing::{debug, instrument};

use crate::CatalogRef;
use crate::lock::build_lock::BuildLock;
use crate::lock::transform::build_lock_from_locked_inputs;

/// Synthetic key for the single wire group the CLI ever sends.
const LOOKUP_GROUP_KEY: &str = "default";

/// Failure modes of [lock_references].
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// The lookup reported unresolvable references. The lock fails as a whole
    /// and no partial lock is produced; each [UnresolvableEntry] carries its
    /// own `reference` and `chain`. Rendering is the caller's responsibility
    /// (ECO-94/A5).
    #[error("{} catalog reference(s) were unresolvable", .0.len())]
    Unresolvable(Vec<UnresolvableEntry>),

    /// The scan produced a root-level sentinel (`catalogs.*`): the expression
    /// uses the whole catalog namespace in a way that cannot be analyzed
    /// statically (e.g. passes `catalogs` to an opaque function or selects a
    /// catalog dynamically). The wire form of such a reference (`*`) can
    /// never resolve, so fail before making a request.
    #[error(
        "Cannot determine which catalogs the expression uses; it references the whole catalog namespace in a way that cannot be analyzed.\n\
         Reference packages as 'catalogs.<catalog>.<package>', or forward 'catalogs' only through 'import' arguments."
    )]
    RootNamespaceEscape,

    /// The catalog lookup request itself failed.
    #[error(transparent)]
    Client(#[from] FloxhubClientError),

    /// Assembling the [BuildLock] from a successful response failed.
    #[error(transparent)]
    Transform(#[from] anyhow::Error),
}

/// Lock a flat list of catalog references in a single batched request.
///
/// Builds the wire request internally (one synthetic group), performs one
/// `/build-inputs/lookup` call, and maps the response to a [BuildLock].
/// Returns [LockError::Unresolvable] if any reference is unresolvable.
///
/// `stability` is the typed [Stability] parsed at the CLI boundary, so this
/// function cannot fail on an invalid stability string.
#[instrument(
    skip(client, references),
    fields(references = references.len(), stability = stability.as_str())
)]
pub async fn lock_references(
    client: &(impl CatalogClientTrait + Send + Sync),
    references: BTreeSet<CatalogRef>,
    stability: Stability,
) -> Result<BuildLock, LockError> {
    if references.iter().any(is_root_sentinel) {
        return Err(LockError::RootNamespaceEscape);
    }

    let request = build_request(references, stability);
    // The exact JSON POSTed to `/build-inputs/lookup`, for `--verbose`. Guarded
    // so the request is only serialized when the level is enabled.
    if tracing::enabled!(tracing::Level::DEBUG) {
        debug!(
            body = %serde_json::to_string(&request)
                .unwrap_or_else(|err| format!("<unserializable request: {err}>")),
            "catalog lookup request",
        );
    }

    let response = client.build_inputs_lookup(request).await?;
    lock_from_response(response)
}

/// Whether a scanned reference is a root-level sentinel (`<root>.*`), whose
/// wire form (`*`) the server can never resolve.
fn is_root_sentinel(reference: &CatalogRef) -> bool {
    reference.as_str().split_once('.').map(|(_root, rest)| rest) == Some("*")
}

/// Convert the reference list into the generated wire request.
///
/// Wraps all references in a single [`floxhub_client::LookupGroup`].
/// `reference_point` is defaulted to `None` for now. The endpoint is
/// system-independent: the response carries source revs + DAG edges, which
/// carry no system, so the request has no system field.
fn build_request(
    references: BTreeSet<CatalogRef>,
    stability: Stability,
) -> BuildInputsLookupRequest {
    let group = LookupGroup {
        key: LOOKUP_GROUP_KEY.to_string(),
        references: references.iter().map(wire_reference).collect(),
    };

    BuildInputsLookupRequest {
        groups: vec![group],
        reference_point: None,
        stability,
    }
}

/// Render a scanned reference for the wire.
///
/// The scanner records references rooted at the NEF `catalogs` lambda parameter
/// (`catalogs.<catalog>.<package>`), but the catalog server's reference
/// namespace is catalog-relative (`<catalog>.<package>`). Drop the leading root
/// segment so the request matches what the server expects.
fn wire_reference(reference: &CatalogRef) -> ReferencesItem {
    let reference = reference.as_str();
    let s = reference
        .split_once('.')
        .map(|(_root, rest)| rest.to_string())
        .unwrap_or_else(|| reference.to_string());
    // Catalog paths are well below the 1024-char wire limit.
    s.parse().expect("catalog reference exceeded 1024 chars")
}

/// Map a lookup response into a [BuildLock], or fail with the unresolvable
/// references.
///
/// The CLI always sends exactly one group, keyed by [LOOKUP_GROUP_KEY], so
/// exactly one group in the response is ours. Extract that group rather than
/// merging across groups; locking multiple groups at once is not supported yet.
///
/// Boundary: if the group reports any unresolvable references, fail the whole
/// lock. Otherwise hand the resolved `lock` map off to the A2 transform.
#[instrument(skip(response))]
fn lock_from_response(mut response: BuildInputsLookupResponse) -> Result<BuildLock, LockError> {
    let Some(group) = response.groups.remove(LOOKUP_GROUP_KEY) else {
        return Err(LockError::Transform(anyhow::anyhow!(
            "The server returned no group for our request; nothing to lock."
        )));
    };

    // Any unresolvable references fail the whole lock with no partial output.
    if !group.unresolvable.is_empty() {
        debug!(
            unresolvable = group.unresolvable.len(),
            "lookup reported unresolvable references"
        );
        return Err(LockError::Unresolvable(group.unresolvable));
    }

    let direct = group.matched.keys();

    debug!(resolved = group.lock.len(), "all references resolved");

    Ok(build_lock_from_locked_inputs(group.lock, direct)?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn root_sentinel_detected_only_at_root_level() {
        let cases = [
            // A root-level sentinel would go on the wire as `*`.
            ("catalogs.*", true),
            // Catalog and package sentinels expand server-side.
            ("catalogs.myorg.*", false),
            ("catalogs.myorg.pkg.*", false),
            ("catalogs.myorg.pkg", false),
        ];
        let got: Vec<(&str, bool)> = cases
            .iter()
            .map(|(reference, _)| (*reference, is_root_sentinel(&CatalogRef::from(*reference))))
            .collect();
        let expected: Vec<(&str, bool)> = cases.into_iter().collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn build_request_maps_references_and_stability() {
        let references = BTreeSet::from([
            CatalogRef::from("catalogs.myorg.hello"),
            CatalogRef::from("catalogs.myorg.world"),
        ]);

        let wire = build_request(references, "stable".parse().unwrap());

        // All references collapse into a single wire group, and the leading
        // `catalogs` root segment is dropped — the server's reference namespace
        // is catalog-relative (`<catalog>.<package>`).
        assert_eq!(wire.groups.len(), 1);
        assert_eq!(
            serde_json::to_value(&wire.groups[0].references).unwrap(),
            json!(["myorg.hello", "myorg.world"])
        );
        assert_eq!(
            serde_json::to_value(&wire.stability).unwrap(),
            json!("stable")
        );
        assert!(wire.reference_point.is_none());
    }

    #[test]
    fn r11_success_fixture_locks() {
        let response: BuildInputsLookupResponse = serde_json::from_str(include_str!(
            "../../test_data/build_inputs_lookup/success.json"
        ))
        .expect("success fixture deserializes");

        let lock = lock_from_response(response).expect("success fixture locks");
        let value = serde_json::to_value(&lock).unwrap();

        assert_eq!(value["version"], json!(1));
        assert_eq!(
            value["catalogs"]["myorg"]["packages"]["entries"]["hello"]["build_type"],
            json!("nef")
        );
        assert_eq!(
            value["catalogs"]["myorg"]["packages"]["entries"]["hello"]["source"],
            json!({
                "type": "git",
                "url": "https://example.com/repo",
                "rev": "abc123",
                "ref": "refs/heads/main",
                "dir": "."
            })
        );
    }

    #[test]
    fn r11_partial_fixture_is_unresolvable() {
        let response: BuildInputsLookupResponse = serde_json::from_str(include_str!(
            "../../test_data/build_inputs_lookup/partial.json"
        ))
        .expect("partial fixture deserializes");

        let err = lock_from_response(response).expect_err("partial fixture fails the lock");

        match err {
            LockError::Unresolvable(entries) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].reference, "myorg.missing-dep");
                assert_eq!(entries[0].chain, vec![
                    "myorg.hello".to_string(),
                    "myorg.missing-dep".to_string(),
                ]);
            },
            other => panic!("expected LockError::Unresolvable, got {other:?}"),
        }
    }
}
