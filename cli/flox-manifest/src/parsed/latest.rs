use crate::interfaces::{AsLatestSchema, AsTypedOnlyManifest, SchemaVersion};
use crate::lockfile::Lockfile;
use crate::parsed::common::KnownSchemaVersion;
pub use crate::parsed::v1_10_0::{
    AllSentinel,
    Install,
    ManifestPackageDescriptor,
    PackageDescriptorCatalog,
    PackageDescriptorFlake,
    SelectedOutputs,
};
use crate::{Manifest, ManifestError, TypedOnly};
pub type ManifestLatest = crate::parsed::v1_10_0::ManifestV1_10_0;

impl ManifestLatest {
    fn as_original_schema(
        &self,
        original_schema: KnownSchemaVersion,
    ) -> Result<Option<Manifest<TypedOnly>>, ManifestError> {
        let mut untyped = serde_json::to_value(self).map_err(ManifestError::SerializeJson)?;
        if self.get_schema_version() != original_schema {
            match original_schema {
                KnownSchemaVersion::V1 => {
                    let map = untyped
                        .as_object_mut()
                        .expect("all valid manifests should serialize to JSON objects");
                    map.remove("schema-version");
                    map.insert("version".into(), 1.into());
                },
                KnownSchemaVersion::V1_10_0 => {},
            }
        }

        let maybe_typed = serde_json::from_value::<Manifest<TypedOnly>>(untyped);
        if maybe_typed.is_err() {
            return Ok(None);
        }
        let Ok(typed_original_schema) = maybe_typed else {
            unreachable!("already checked that deserialization succeeded");
        };
        Ok(Some(typed_original_schema))
    }

    pub fn as_maybe_backwards_compatible(
        &self,
        original_schema: KnownSchemaVersion,
        lockfile: Option<&Lockfile>,
    ) -> Result<Manifest<TypedOnly>, ManifestError> {
        let maybe_backwards_compatible = self.as_original_schema(original_schema)?;
        if maybe_backwards_compatible.is_none() {
            // If this was `None` it means we couldn't represent the current
            // manifest in the old schema at all (there could be new fields,
            // syntax, etc). In that case, we *must* migrate.
            return Ok(self.as_typed_only());
        }
        let backwards_compatible =
            maybe_backwards_compatible.expect("just verified that option is some");
        let migrated_again = backwards_compatible.migrate_typed_only(lockfile)?;
        let migrated_again = migrated_again.as_latest_schema();
        if migrated_again == self {
            Ok(backwards_compatible)
        } else {
            Ok(self.as_typed_only())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use flox_core::data::environment_ref::RemoteEnvironmentRef;
    use indoc::{formatdoc, indoc};
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;
    use serde::Deserialize;

    use super::*;
    use crate::interfaces::PackageLookup;
    use crate::parsed::common::{
        Build,
        BuildDescriptor,
        BuildVersion,
        Hook,
        IncludeDescriptor,
        PackageDescriptorStorePath,
        Profile,
    };
    use crate::parsed::Inner;
    use crate::test_helpers::with_latest_schema;
    use crate::ManifestError;

    #[test]
    fn catalog_manifest_rejects_unknown_fields() {
        let manifest = with_latest_schema("unknown = 'field'");

        let err = toml_edit::de::from_str::<ManifestLatest>(&manifest)
            .expect_err("manifest.toml should be invalid");

        assert!(
            err.message()
                .starts_with("unknown field `unknown`, expected one of"),
            "unexpected error message: {err}",
        );
    }

    #[test]
    fn catalog_manifest_rejects_unknown_nested_fields() {
        let manifest = with_latest_schema(formatdoc! {"
            [options]
            allow.unknown = true
        "});

        let err = toml_edit::de::from_str::<ManifestLatest>(&manifest)
            .expect_err("manifest.toml should be invalid");

        assert!(
            err.message()
                .starts_with("unknown field `unknown`, expected one of"),
            "unexpected error message: {err}",
        );
    }

    #[test]
    fn detect_catalog_manifest() {
        assert!(toml_edit::de::from_str::<ManifestLatest>(with_latest_schema("").as_str()).is_ok());
    }

    // FIXME
    // proptest! {
    //     #[test]
    //     fn manifest_round_trip(manifest in any::<ManifestV1_10_0>()) {
    //         let toml = toml_edit::ser::to_string(&manifest).unwrap();
    //         let parsed = toml_edit::de::from_str::<ManifestV1_10_0>(&toml).unwrap();
    //         prop_assert_eq!(manifest, parsed);
    //     }

    //     #[test]
    //     fn manifest_from_str_round_trip(manifest in any::<ManifestV1_10_0>()) {
    //         let toml = toml_edit::ser::to_string(&manifest).unwrap();
    //         let parsed = ManifestV1_10_0::from_str(&toml).unwrap();
    //         prop_assert_eq!(manifest, parsed);
    //     }
    // }

    fn has_null_fields(json_str: &str) -> bool {
        type Value = serde_json::Value;
        let json_value: Value = serde_json::from_str(json_str).unwrap();

        // Recursively check if any field in the JSON is `null`
        fn check_for_null(value: &Value) -> bool {
            match value {
                Value::Null => true,
                Value::Object(map) => map.values().any(check_for_null),
                Value::Array(arr) => arr.iter().any(check_for_null),
                _ => false,
            }
        }

        check_for_null(&json_value)
    }

    // Null fields rendered into the lockfile cause backwards-compatibility issues for new fields.
    proptest! {
        #[test]
        fn manifest_does_not_serialize_null_fields(manifest in any::<ManifestLatest>()) {
            let json_str = serde_json::to_string_pretty(&manifest).unwrap();
            prop_assert!(!has_null_fields(&json_str), "json: {}", &json_str);
        }
    }

    // A serialized manifest shouldn't contain any tables that aren't specified
    // or required, with the exception of `options` which is fiddly to implement
    // `skip_serializing_if` for such a mixture of fields.
    //
    // This makes the lockfile tidier and improve cross-version compatibility.
    // It doesn't affect the presentation of composed manifests because `flox
    // list` uses a different serializer.
    #[test]
    fn serialize_omits_unspecified_fields() {
        let manifest = ManifestLatest::default();
        let expected = with_latest_schema("[options]");

        let actual = toml_edit::ser::to_string_pretty(&manifest).unwrap();
        assert_eq!(actual, expected);
    }

    // If a user specifies an uncommented `[hook]` or `[profile]` table without
    // any contents, like the manifest template does, then we preserve that in
    // the serialized output.
    #[test]
    fn serialize_preserves_explicitly_empty_tables() {
        let manifest = ManifestLatest {
            hook: Some(Hook::default()),
            profile: Some(Profile::default()),
            ..Default::default()
        };
        let expected = with_latest_schema(indoc! {r#"
            [hook]

            [profile]

            [options]"#});

        let actual = toml_edit::ser::to_string_pretty(&manifest).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn parses_build_section() {
        let build_manifest = with_latest_schema(indoc! {r#"
            [build]
            test.command = 'hello'

        "#});

        let parsed = toml_edit::de::from_str::<ManifestLatest>(&build_manifest).unwrap();

        assert_eq!(
            parsed.build,
            Build(
                [("test".to_string(), BuildDescriptor {
                    command: "hello".to_string(),
                    runtime_packages: None,
                    sandbox: None,
                    version: None,
                    description: None,
                    license: None,
                })]
                .into()
            )
        );
    }

    #[test]
    fn parses_version() {
        #[derive(Deserialize)]
        struct VersionWrap {
            version: BuildVersion,
        }
        let parse =
            |version| toml_edit::de::from_str::<VersionWrap>(version).map(|wrap| wrap.version);

        assert_eq!(
            parse("version = '1.2.3'"),
            Ok(BuildVersion::Pure("1.2.3".into()))
        );
        assert_eq!(
            parse("version.file = 'FILE'"),
            Ok(BuildVersion::File {
                file: "FILE".into()
            })
        );
        assert_eq!(
            parse("version.command = 'command'"),
            Ok(BuildVersion::Command {
                command: "command".into()
            })
        );
        assert!(parse("other = 'wont parse'").is_err())
    }

    #[test]
    fn filter_services_by_system() {
        let manifest = with_latest_schema(indoc! {r#"
            [services]
            postgres.command = "postgres"
            mysql.command = "mysql"
            mysql.systems = ["x86_64-linux", "aarch64-linux"]
            redis.command = "redis"
            redis.systems = ["aarch64-linux"]
        "#});

        let parsed = toml_edit::de::from_str::<ManifestLatest>(&manifest).unwrap();

        assert_eq!(parsed.services.inner().len(), 3, "{:?}", parsed.services);

        let filtered = parsed.services.copy_for_system(&"x86_64-linux".to_string());
        assert_eq!(filtered.inner().len(), 2, "{:?}", filtered);
        assert!(filtered.inner().contains_key("postgres"));
        assert!(filtered.inner().contains_key("mysql"));

        let filtered = parsed
            .services
            .copy_for_system(&"aarch64-darwin".to_string());
        assert_eq!(filtered.inner().len(), 1, "{:?}", filtered);
        assert!(filtered.inner().contains_key("postgres"));
    }

    #[test]
    fn parses_include_section_manifest() {
        let manifest = with_latest_schema(indoc! {r#"
            [include]
            environments = [
                { dir = "../foo", name = "bar" },
                { remote = "owner/repo", name = "baz" },
                # reference alias for remote
                { reference = "owner/repo", name = "bap" },
            ]
        "#});
        let parsed = toml_edit::de::from_str::<ManifestLatest>(&manifest).unwrap();

        assert_eq!(parsed.include.environments, vec![
            IncludeDescriptor::Local {
                dir: PathBuf::from("../foo"),
                name: Some("bar".to_string()),
            },
            IncludeDescriptor::Remote {
                remote: RemoteEnvironmentRef::new("owner", "repo").unwrap(),
                name: Some("baz".to_string()),
                generation: None,
            },
            IncludeDescriptor::Remote {
                remote: RemoteEnvironmentRef::new("owner", "repo").unwrap(),
                name: Some("bap".to_string()),
                generation: None,
            },
        ]);
    }

    /// Generates a mock `TypedManifest` for testing purposes.
    /// This function is designed to simplify the creation of test data by
    /// generating a `TypedManifest` based on a list of install IDs and
    /// package paths.
    /// # Arguments
    ///
    /// * `entries` - A vector of tuples, where each tuple contains an install
    ///   ID and a package path.
    ///
    /// # Returns
    ///
    /// * `TypedManifest` - A mock `TypedManifest` containing the provided entries.
    fn generate_mock_manifest(entries: Vec<(&str, &str)>) -> ManifestLatest {
        let mut typed_manifest_mock = ManifestLatest::default();

        for (test_iid, dotted_package) in entries {
            typed_manifest_mock.install.inner_mut().insert(
                test_iid.to_string(),
                ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog {
                    pkg_path: dotted_package.to_string(),
                    pkg_group: None,
                    priority: None,
                    version: None,
                    systems: None,
                    outputs: None,
                }),
            );
        }

        typed_manifest_mock
    }
    /// Return the install ID if it matches the user input
    #[test]
    fn test_get_install_ids_to_uninstall_by_install_id() {
        let manifest_mock = generate_mock_manifest(vec![("testInstallID", "dotted.package")]);
        let result = manifest_mock
            .get_install_ids(vec!["testInstallID".to_string()])
            .unwrap();
        assert_eq!(result, vec!["testInstallID".to_string()]);
    }

    #[test]
    /// Return the install ID if a pkg-path matches the user input
    fn test_get_install_ids_to_uninstall_by_pkg_path() {
        let manifest_mock = generate_mock_manifest(vec![("testInstallID", "dotted.package")]);
        let result = manifest_mock
            .get_install_ids(vec!["dotted.package".to_string()])
            .unwrap();
        assert_eq!(result, vec!["testInstallID".to_string()]);
    }

    #[test]
    /// Ensure that the install ID takes precedence over pkg-path when both are present
    fn test_get_install_ids_to_uninstall_iid_wins() {
        let manifest_mock = generate_mock_manifest(vec![
            ("testInstallID1", "dotted.package"),
            ("testInstallID2", "dotted.package"),
            ("dotted.package", "dotted.package"),
        ]);

        let result = manifest_mock
            .get_install_ids(vec!["dotted.package".to_string()])
            .unwrap();
        assert_eq!(result, vec!["dotted.package".to_string()]);
    }

    #[test]
    /// Throw an error when multiple packages match by pkg_path and flox can't determine which to uninstall
    fn test_get_install_ids_to_uninstall_multiple_pkg_paths_match() {
        let manifest_mock = generate_mock_manifest(vec![
            ("testInstallID1", "dotted.package"),
            ("testInstallID2", "dotted.package"),
            ("testInstallID3", "dotted.package"),
        ]);
        let result = manifest_mock
            .get_install_ids(vec!["dotted.package".to_string()])
            .unwrap_err();
        assert!(matches!(result, ManifestError::MultiplePackagesMatch(_, _)));
    }

    #[test]
    /// Throw an error if no install ID or pkg-path matches the user input
    fn test_get_install_ids_to_uninstall_pkg_not_found() {
        let manifest_mock = generate_mock_manifest(vec![("testInstallID1", "dotted.package")]);
        let result = manifest_mock
            .get_install_ids(vec!["invalid.packageName".to_string()])
            .unwrap_err();
        assert!(matches!(result, ManifestError::PackageNotFound(_)));
    }

    #[test]
    fn test_get_install_ids_to_uninstall_with_version() {
        let mut manifest_mock = generate_mock_manifest(vec![("testInstallID", "dotted.package")]);

        if let ManifestPackageDescriptor::Catalog(descriptor) = manifest_mock
            .install
            .inner_mut()
            .get_mut("testInstallID")
            .unwrap()
        {
            descriptor.version = Some("1.0".to_string());
        };

        let result = manifest_mock
            .get_install_ids(vec!["dotted.package@1.0".to_string()])
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "testInstallID");
    }

    /// Helper function to create a catalog descriptor for testing
    fn create_catalog_descriptor(pkg_path: &str) -> ManifestPackageDescriptor {
        ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog {
            pkg_path: pkg_path.to_string(),
            pkg_group: None,
            priority: None,
            version: None,
            systems: None,
            outputs: None,
        })
    }

    /// Helper function to create a flake descriptor for testing
    fn create_flake_descriptor(flake: &str) -> ManifestPackageDescriptor {
        ManifestPackageDescriptor::FlakeRef(PackageDescriptorFlake {
            flake: flake.to_string(),
            priority: None,
            systems: None,
            outputs: None,
        })
    }

    /// Helper function to create a store path descriptor for testing
    fn create_store_path_descriptor(store_path: &str) -> ManifestPackageDescriptor {
        ManifestPackageDescriptor::StorePath(PackageDescriptorStorePath {
            store_path: store_path.to_string(),
            systems: None,
            priority: None,
        })
    }

    #[test]
    fn test_is_from_custom_catalog() {
        assert!(!create_catalog_descriptor("hello").is_from_custom_catalog());
        assert!(create_catalog_descriptor("mycatalog/hello").is_from_custom_catalog());

        // Test non-catalog descriptors always return false
        assert!(!create_flake_descriptor("github:owner/repo").is_from_custom_catalog());
        assert!(!create_store_path_descriptor("/nix/store/abc123-hello").is_from_custom_catalog());
    }
}
