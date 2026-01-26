use std::collections::BTreeMap;

#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::btree_map_strategy;
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::interfaces::{AsTypedOnlyManifest, CommonFields, SchemaVersion, impl_pkg_lookup};
use crate::parsed::common::{
    Build,
    Containerize,
    Hook,
    Include,
    KnownSchemaVersion,
    Options,
    Profile,
    Services,
    Vars,
};
use crate::parsed::{Inner, SkipSerializing, impl_into_inner};
use crate::{Manifest, ManifestError, Parsed, TypedOnly};

mod package_descriptor;
pub use package_descriptor::*;

/// Not meant for writing manifest files, only for reading them.
/// Modifications should be made using `manifest::raw`.

// We use `skip_serializing_none` and `skip_serializing_if` throughout to reduce
// the size of the lockfile and improve backwards compatibility when we
// introduce fields.
//
// It would be better if we could deny_unknown_fields when we're deserializing
// the user provided manifest but allow unknown fields when deserializing the
// lockfile, but that doesn't seem worth the effort at the moment.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct ManifestV1 {
    pub version: ManifestVersion,
    /// The packages to install in the form of a map from install_id
    /// to package descriptor.
    #[serde(default)]
    #[serde(skip_serializing_if = "Install::skip_serializing")]
    pub install: Install,
    /// Variables that are exported to the shell environment upon activation.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vars::skip_serializing")]
    pub vars: Vars,
    /// Hooks that are run at various times during the lifecycle of the manifest
    /// in a known shell environment.
    #[serde(default)]
    pub hook: Option<Hook>,
    /// Profile scripts that are run in the user's shell upon activation.
    #[serde(default)]
    pub profile: Option<Profile>,
    /// Options that control the behavior of the manifest.
    #[serde(default)]
    pub options: Options,
    /// Service definitions
    #[serde(default)]
    #[serde(skip_serializing_if = "Services::skip_serializing")]
    pub services: Services,
    /// Package build definitions
    #[serde(default)]
    #[serde(skip_serializing_if = "Build::skip_serializing")]
    pub build: Build,
    #[serde(default)]
    pub containerize: Option<Containerize>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Include::skip_serializing")]
    pub include: Include,
}
impl_pkg_lookup!(crate::parsed::v1, ManifestV1);

impl AsTypedOnlyManifest for ManifestV1 {
    fn as_typed_only(&self) -> crate::Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::V1(self.clone()),
            },
        }
    }
}

impl SchemaVersion for ManifestV1 {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        KnownSchemaVersion::V1
    }
}

impl CommonFields for ManifestV1 {
    fn vars(&self) -> &Vars {
        &self.vars
    }

    fn hook(&self) -> Option<&Hook> {
        self.hook.as_ref()
    }

    fn profile(&self) -> Option<&Profile> {
        self.profile.as_ref()
    }

    fn services(&self) -> &Services {
        &self.services
    }

    fn include(&self) -> &Include {
        &self.include
    }

    fn build(&self) -> &Build {
        &self.build
    }

    fn containerize(&self) -> Option<&super::common::Containerize> {
        self.containerize.as_ref()
    }

    fn options(&self) -> &super::common::Options {
        &self.options
    }

    fn vars_mut(&mut self) -> &mut super::common::Vars {
        &mut self.vars
    }

    fn hook_mut(&mut self) -> Option<&mut super::common::Hook> {
        self.hook.as_mut()
    }

    fn profile_mut(&mut self) -> Option<&mut super::common::Profile> {
        self.profile.as_mut()
    }

    fn services_mut(&mut self) -> &mut super::common::Services {
        &mut self.services
    }

    fn include_mut(&mut self) -> &mut super::common::Include {
        &mut self.include
    }

    fn build_mut(&mut self) -> &mut super::common::Build {
        &mut self.build
    }

    fn containerize_mut(&mut self) -> Option<&mut super::common::Containerize> {
        self.containerize.as_mut()
    }

    fn options_mut(&mut self) -> &mut super::common::Options {
        &mut self.options
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct ManifestVersion(u8);

impl Default for ManifestVersion {
    fn default() -> Self {
        Self(1)
    }
}

#[cfg(any(test, feature = "tests"))]
impl Arbitrary for ManifestVersion {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
        prop_oneof!(Just(ManifestVersion(1)), Just(ManifestVersion(2)),).boxed()
    }
}

impl_into_inner!(ManifestVersion, u8);

impl From<u8> for ManifestVersion {
    fn from(value: u8) -> Self {
        ManifestVersion(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct Install(
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "btree_map_strategy::<ManifestPackageDescriptor>(10, 3)")
    )]
    pub(crate) BTreeMap<String, ManifestPackageDescriptor>,
);

impl SkipSerializing for Install {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

impl_into_inner!(Install, BTreeMap<String, ManifestPackageDescriptor>);

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use super::*;

    // Generate a Manifest that has empty install and include sections
    pub fn manifest_without_install_or_include() -> impl Strategy<Value = ManifestV1> {
        (
            any::<ManifestVersion>(),
            any::<Vars>(),
            any::<Option<Hook>>(),
            any::<Option<Profile>>(),
            any::<Options>(),
            any::<Services>(),
            any::<Build>(),
            any::<Option<Containerize>>(),
        )
            .prop_map(
                |(version, vars, hook, profile, options, services, build, containerize)| {
                    ManifestV1 {
                        version,
                        install: Install::default(),
                        vars,
                        hook,
                        profile,
                        options,
                        services,
                        build,
                        containerize,
                        include: Include::default(),
                    }
                },
            )
    }
}

#[cfg(test)]
pub mod test {
    use std::path::PathBuf;

    use flox_core::data::environment_ref::RemoteEnvironmentRef;
    use indoc::{formatdoc, indoc};
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;

    use super::*;
    use crate::interfaces::PackageLookup;
    use crate::parsed::common::{
        BuildDescriptor,
        BuildVersion,
        IncludeDescriptor,
        PackageDescriptorStorePath,
    };
    use crate::parsed::v1::package_descriptor::{PackageDescriptorCatalog, PackageDescriptorFlake};

    const CATALOG_MANIFEST: &str = indoc! {r#"
        version = 1
    "#};

    #[test]
    fn catalog_manifest_rejects_unknown_fields() {
        let manifest = formatdoc! {"
            {CATALOG_MANIFEST}

            unknown = 'field'
        "};

        let err = toml_edit::de::from_str::<ManifestV1>(&manifest)
            .expect_err("manifest.toml should be invalid");

        assert!(
            err.message()
                .starts_with("unknown field `unknown`, expected one of"),
            "unexpected error message: {err}",
        );
    }

    #[test]
    fn catalog_manifest_rejects_unknown_nested_fields() {
        let manifest = formatdoc! {"
            {CATALOG_MANIFEST}

            [options]
            allow.unknown = true
        "};

        let err = toml_edit::de::from_str::<ManifestV1>(&manifest)
            .expect_err("manifest.toml should be invalid");

        assert!(
            err.message()
                .starts_with("unknown field `unknown`, expected one of"),
            "unexpected error message: {err}",
        );
    }

    #[test]
    fn detect_catalog_manifest() {
        assert!(toml_edit::de::from_str::<ManifestV1>(CATALOG_MANIFEST).is_ok());
    }

    // FIXME: these tests will need to be rewritten to use the helper functions
    //        on the Manifest type
    // proptest! {
    //     #[test]
    //     fn manifest_round_trip(manifest in any::<ManifestV1>()) {
    //         let toml = toml_edit::ser::to_string(&manifest).unwrap();
    //         let parsed = toml_edit::de::from_str::<ManifestV1>(&toml).unwrap();
    //         prop_assert_eq!(manifest, parsed);
    //     }

    //     #[test]
    //     fn manifest_from_str_round_trip(manifest in any::<ManifestV1>()) {
    //         let toml = toml_edit::ser::to_string(&manifest).unwrap();
    //         let parsed = ManifestV1::from_str(&toml).unwrap();
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
        fn manifest_does_not_serialize_null_fields(manifest in any::<ManifestV1>()) {
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
        let manifest = ManifestV1::default();
        let expected = indoc! {r#"
            version = 1

            [options]
        "#};

        let actual = toml_edit::ser::to_string_pretty(&manifest).unwrap();
        assert_eq!(actual, expected);
    }

    // If a user specifies an uncommented `[hook]` or `[profile]` table without
    // any contents, like the manifest template does, then we preserve that in
    // the serialized output.
    #[test]
    fn serialize_preserves_explicitly_empty_tables() {
        let manifest = ManifestV1 {
            hook: Some(Hook::default()),
            profile: Some(Profile::default()),
            ..Default::default()
        };
        let expected = indoc! {r#"
            version = 1

            [hook]

            [profile]

            [options]
        "#};

        let actual = toml_edit::ser::to_string_pretty(&manifest).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn parses_build_section() {
        let build_manifest = indoc! {r#"
            version = 1
            [build]
            test.command = 'hello'

        "#};

        let parsed = toml_edit::de::from_str::<ManifestV1>(build_manifest).unwrap();

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
        let manifest = indoc! {r#"
            version = 1
            [services]
            postgres.command = "postgres"
            mysql.command = "mysql"
            mysql.systems = ["x86_64-linux", "aarch64-linux"]
            redis.command = "redis"
            redis.systems = ["aarch64-linux"]
        "#};

        let parsed = toml_edit::de::from_str::<ManifestV1>(manifest).unwrap();

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
        let manifest = indoc! {r#"
            version = 1

            [include]
            environments = [
                { dir = "../foo", name = "bar" },
                { remote = "owner/repo", name = "baz" },
                # reference alias for remote
                { reference = "owner/repo", name = "bap" },
            ]
        "#};
        let parsed = toml_edit::de::from_str::<ManifestV1>(manifest).unwrap();

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
    fn generate_mock_manifest(entries: Vec<(&str, &str)>) -> ManifestV1 {
        let mut typed_manifest_mock = ManifestV1::default();

        for (test_iid, dotted_package) in entries {
            typed_manifest_mock.install.inner_mut().insert(
                test_iid.to_string(),
                ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog {
                    pkg_path: dotted_package.to_string(),
                    pkg_group: None,
                    priority: None,
                    version: None,
                    systems: None,
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
        })
    }

    /// Helper function to create a flake descriptor for testing
    fn create_flake_descriptor(flake: &str) -> ManifestPackageDescriptor {
        ManifestPackageDescriptor::FlakeRef(PackageDescriptorFlake {
            flake: flake.to_string(),
            priority: None,
            systems: None,
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
