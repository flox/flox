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
    BuildInputsLookupResponseV2,
    CatalogClientTrait,
    DEFAULT_STABILITY,
    FloxhubClientError,
    LookupGroup,
    ReferencesItem,
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

    /// The catalog lookup request itself failed.
    #[error(transparent)]
    Client(#[from] FloxhubClientError),

    /// Assembling the [BuildLock] from a successful response failed.
    #[error(transparent)]
    Transform(#[from] anyhow::Error),
}

/// Lock a flat list of catalog references in a single request.
///
/// Builds the wire request internally (one synthetic group), performs one
/// `/build-inputs/lookup` call, and maps the response to a [BuildLock].
/// Returns [LockError::Unresolvable] if any reference is unresolvable.
#[instrument(skip(client, references), fields(references = references.len()))]
pub async fn lock_references(
    client: &impl CatalogClientTrait,
    references: BTreeSet<CatalogRef>,
) -> Result<BuildLock, LockError> {
    let request = build_request(references);
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

/// Convert the reference list into the generated wire request.
///
/// Wraps all references in a single [`floxhub_client::LookupGroup`].
/// `reference_point` is defaulted to `None` for now. The endpoint is
/// system-independent: the response carries source revs + DAG edges, which
/// carry no system, so the request has no system field.
fn build_request(references: BTreeSet<CatalogRef>) -> BuildInputsLookupRequest {
    let group = LookupGroup {
        key: LOOKUP_GROUP_KEY.to_string(),
        references: references.iter().map(wire_reference).collect(),
    };

    BuildInputsLookupRequest {
        groups: vec![group],
        reference_point: None,
        response_version: 2.try_into().expect("supported lookup response version"),
        // Catalog-input resolution is independent of the nixpkgs base-catalog
        // stability. The server accepts and ignores the field, and the spec
        // marks it deprecated, but older servers still read it, so send the
        // default stability until the field is dropped from the spec.
        stability: Some(DEFAULT_STABILITY.to_owned()),
    }
}

/// Render a scanned reference for the wire.
///
/// The scanner records references rooted at the NEF `catalogs` lambda parameter
/// (`catalogs.<catalog>.<package>`), but the catalog server's reference
/// namespace is catalog-relative (`<catalog>.<package>`). Drop the leading root
/// segment so the request matches what the server expects.
fn wire_reference(reference: &CatalogRef) -> ReferencesItem {
    // An attribute Nix would not read bare goes out quoted, which the dotted
    // wire format needs to stay unambiguous — it cannot express a name
    // containing a `.` any other way.
    //
    // Catalog paths are well below the 1024-char wire limit.
    reference
        .wire_key()
        .parse()
        .expect("catalog reference exceeded 1024 chars")
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
fn lock_from_response(mut response: BuildInputsLookupResponseV2) -> Result<BuildLock, LockError> {
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

    for reference in group.not_lockable.keys() {
        if reference != "nixpkgs" && !reference.starts_with("nixpkgs.") {
            return Err(LockError::Transform(anyhow::anyhow!(
                "catalog reference '{reference}' cannot be locked"
            )));
        }
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
    fn build_request_maps_references() {
        let references = BTreeSet::from([
            CatalogRef::new_unchecked("catalogs.myorg.hello"),
            CatalogRef::new_unchecked("catalogs.myorg.world"),
        ]);

        let wire = build_request(references);

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
            json!(DEFAULT_STABILITY)
        );
        assert!(wire.reference_point.is_none());
        assert_eq!(*wire.response_version, 2);
    }

    #[test]
    fn r11_success_fixture_locks() {
        let response: BuildInputsLookupResponseV2 = serde_json::from_str(include_str!(
            "../../test_data/build_inputs_lookup/success.json"
        ))
        .expect("success fixture deserializes");

        let lock = lock_from_response(response).expect("success fixture locks");
        let value: serde_json::Value =
            serde_json::from_str(&crate::lock::transform::render_builder_lock(&lock).unwrap())
                .unwrap();

        assert_eq!(value["version"], json!(2));
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

    /// The server's canonical keys and dotted reference names must not be conflated.
    #[test]
    fn success_fixture_projects_by_its_own_reference() {
        let response: BuildInputsLookupResponseV2 = serde_json::from_str(include_str!(
            "../../test_data/build_inputs_lookup/success.json"
        ))
        .expect("success fixture deserializes");
        let lock = lock_from_response(response).expect("success fixture locks");

        let references = BTreeSet::from([CatalogRef::new_unchecked("catalogs.myorg.hello")]);
        let closure = lock
            .project_package(&references)
            .expect("the lock's own reference is covered");

        assert_eq!(closure.direct_inputs, vec!["myorg/hello".to_string()]);
        assert_eq!(
            closure.locked_inputs["myorg/hello"],
            floxhub_client::LockedInputEntry::from(&lock.locked_inputs["myorg/hello"])
        );
    }

    #[test]
    fn wire_nar_hash_survives_write_and_read() {
        let mut wire: serde_json::Value = serde_json::from_str(include_str!(
            "../../test_data/build_inputs_lookup/success.json"
        ))
        .unwrap();
        wire["version"] = json!(2);
        wire["groups"][LOOKUP_GROUP_KEY]["lock"]["myorg/hello"]["source"]["narHash"] =
            json!("sha256-wire-extra");
        let response: BuildInputsLookupResponseV2 = serde_json::from_value(wire).unwrap();
        let lock = lock_from_response(response).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.lock");
        crate::write_lock(&lock, &path).unwrap();
        let read = crate::read_lock(&path).unwrap();
        assert_eq!(
            read.locked_inputs["myorg/hello"].source.extra["narHash"],
            json!("sha256-wire-extra")
        );
        let closure = read
            .project_package(&BTreeSet::from([CatalogRef::new_unchecked(
                "catalogs.myorg.hello",
            )]))
            .unwrap();
        assert_eq!(
            closure.locked_inputs["myorg/hello"].source.extra["narHash"],
            json!("sha256-wire-extra")
        );
    }

    #[test]
    fn r11_partial_fixture_is_unresolvable() {
        let response: BuildInputsLookupResponseV2 = serde_json::from_str(include_str!(
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

    #[test]
    fn v2_base_only_wire_locks_without_a_root() {
        let response: BuildInputsLookupResponseV2 = serde_json::from_value(json!({
            "version": 2,
            "groups": {"default": {
                "lock": {}, "matched": {}, "unresolvable": [],
                "not_lockable": {"nixpkgs.python3Packages.*": {"kind": "base_catalog"}}
            }}
        }))
        .expect("v2 model parses the wire response");
        assert_eq!(response.version, Some(2));
        assert_eq!(
            response.groups[LOOKUP_GROUP_KEY].not_lockable["nixpkgs.python3Packages.*"].kind,
            "base_catalog"
        );

        let lock = lock_from_response(response).expect("base-only group is usable");
        assert!(lock.direct_inputs.is_empty());
        assert!(lock.locked_inputs.is_empty());
        assert_eq!(
            crate::lock::transform::materialize_catalogs(&lock).unwrap(),
            json!({})
        );
    }

    #[test]
    fn v2_mixed_group_keeps_only_lockable_roots() {
        let mut response: serde_json::Value = serde_json::from_str(include_str!(
            "../../test_data/build_inputs_lookup/success.json"
        ))
        .unwrap();
        response["version"] = json!(2);
        response["groups"][LOOKUP_GROUP_KEY]["not_lockable"] =
            json!({"nixpkgs.hello": {"kind": "base_catalog"}});
        let response: BuildInputsLookupResponseV2 = serde_json::from_value(response).unwrap();

        let lock = lock_from_response(response).expect("mixed group is usable");
        assert_eq!(
            lock.direct_inputs,
            BTreeSet::from(["myorg/hello".to_string()])
        );
        assert_eq!(lock.locked_inputs.len(), 1);
        let value: serde_json::Value =
            serde_json::from_str(&crate::lock::transform::render_builder_lock(&lock).unwrap())
                .unwrap();
        assert!(value["catalogs"].get("nixpkgs").is_none());
    }

    #[test]
    fn not_lockable_outside_base_namespace_fails_the_lock() {
        let response: BuildInputsLookupResponseV2 = serde_json::from_value(json!({
            "version": 2,
            "groups": {"default": {
                "lock": {}, "matched": {},
                "not_lockable": {"other.hello": {"kind": "future_kind"}}
            }}
        }))
        .unwrap();
        let error = lock_from_response(response).unwrap_err().to_string();
        assert!(error.contains("other.hello"), "{error}");
    }

    #[test]
    fn v2_unresolvable_still_fails_with_a_base_advisory() {
        let mut response: serde_json::Value = serde_json::from_str(include_str!(
            "../../test_data/build_inputs_lookup/partial.json"
        ))
        .unwrap();
        response["version"] = json!(2);
        response["groups"][LOOKUP_GROUP_KEY]["not_lockable"] =
            json!({"nixpkgs.hello": {"kind": "base_catalog"}});
        let response: BuildInputsLookupResponseV2 = serde_json::from_value(response).unwrap();
        assert!(matches!(
            lock_from_response(response),
            Err(LockError::Unresolvable(entries)) if entries.len() == 1
        ));
    }
}
