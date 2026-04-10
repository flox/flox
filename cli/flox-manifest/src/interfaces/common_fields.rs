use flox_core::activate::mode::ActivateMode;
use flox_core::data::System;

use crate::{Manifest, Migrated, MigratedTypedOnly, Parsed, TypedOnly, Validated, parsed};

/// Accessors for fields that are shared across all schema versions.
///
/// The trait historically exposed `options()` / `options_mut()` returning
/// `&parsed::common::Options`, but that could not accommodate schema versions
/// that need to store version-local types nested inside `options` (e.g.
/// `options.activate.add-sbin` in V1_12_0). Instead the trait exposes
/// per-field accessors for each option that is actually read or written by
/// callers. New option fields added in later schema versions should be
/// surfaced as new trait methods with default impls returning `None`/`false`
/// so that pre-existing schema versions remain unchanged.
pub trait CommonFields {
    fn vars(&self) -> &parsed::common::Vars;
    fn hook(&self) -> Option<&parsed::common::Hook>;
    fn profile(&self) -> Option<&parsed::common::Profile>;
    fn services(&self) -> &parsed::common::Services;
    fn include(&self) -> &parsed::common::Include;
    fn build(&self) -> &parsed::common::Build;
    fn containerize(&self) -> Option<&parsed::common::Containerize>;

    // `options.*` accessors.
    fn systems(&self) -> Option<&Vec<System>>;
    fn allows(&self) -> &parsed::common::Allows;
    fn semver_options(&self) -> &parsed::common::SemverOptions;
    fn cuda_detection(&self) -> Option<bool>;
    fn activate_mode(&self) -> Option<&ActivateMode>;

    /// Returns whether services should auto-start on activation.
    ///
    /// Returns `false` for all schema versions before V1_12_0.
    /// V1_12_0 and later read this from the manifest's `[services] auto-start`
    /// field.
    fn services_auto_start(&self) -> bool {
        false
    }

    fn vars_mut(&mut self) -> &mut parsed::common::Vars;
    fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook>;
    fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile>;
    fn services_mut(&mut self) -> &mut parsed::common::Services;
    fn include_mut(&mut self) -> &mut parsed::common::Include;
    fn build_mut(&mut self) -> &mut parsed::common::Build;
    fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize>;

    // `options.*` mutable accessors.
    fn systems_mut(&mut self) -> &mut Option<Vec<System>>;
    fn activate_mode_mut(&mut self) -> &mut Option<ActivateMode>;
}

impl CommonFields for Parsed {
    fn vars(&self) -> &parsed::common::Vars {
        match self {
            Parsed::V1(inner) => inner.vars(),
            Parsed::V1_10_0(inner) => inner.vars(),
            Parsed::V1_11_0(inner) => inner.vars(),
            Parsed::V1_12_0(inner) => inner.vars(),
        }
    }

    fn hook(&self) -> Option<&parsed::common::Hook> {
        match self {
            Parsed::V1(inner) => inner.hook(),
            Parsed::V1_10_0(inner) => inner.hook(),
            Parsed::V1_11_0(inner) => inner.hook(),
            Parsed::V1_12_0(inner) => inner.hook(),
        }
    }

    fn profile(&self) -> Option<&parsed::common::Profile> {
        match self {
            Parsed::V1(inner) => inner.profile(),
            Parsed::V1_10_0(inner) => inner.profile(),
            Parsed::V1_11_0(inner) => inner.profile(),
            Parsed::V1_12_0(inner) => inner.profile(),
        }
    }

    fn services(&self) -> &parsed::common::Services {
        match self {
            Parsed::V1(inner) => inner.services(),
            Parsed::V1_10_0(inner) => inner.services(),
            Parsed::V1_11_0(inner) => inner.services(),
            Parsed::V1_12_0(inner) => inner.services(),
        }
    }

    fn include(&self) -> &parsed::common::Include {
        match self {
            Parsed::V1(inner) => inner.include(),
            Parsed::V1_10_0(inner) => inner.include(),
            Parsed::V1_11_0(inner) => inner.include(),
            Parsed::V1_12_0(inner) => inner.include(),
        }
    }

    fn build(&self) -> &parsed::common::Build {
        match self {
            Parsed::V1(inner) => inner.build(),
            Parsed::V1_10_0(inner) => inner.build(),
            Parsed::V1_11_0(inner) => inner.build(),
            Parsed::V1_12_0(inner) => inner.build(),
        }
    }

    fn containerize(&self) -> Option<&parsed::common::Containerize> {
        match self {
            Parsed::V1(inner) => inner.containerize(),
            Parsed::V1_10_0(inner) => inner.containerize(),
            Parsed::V1_11_0(inner) => inner.containerize(),
            Parsed::V1_12_0(inner) => inner.containerize(),
        }
    }

    fn systems(&self) -> Option<&Vec<System>> {
        match self {
            Parsed::V1(inner) => inner.systems(),
            Parsed::V1_10_0(inner) => inner.systems(),
            Parsed::V1_11_0(inner) => inner.systems(),
            Parsed::V1_12_0(inner) => inner.systems(),
        }
    }

    fn allows(&self) -> &parsed::common::Allows {
        match self {
            Parsed::V1(inner) => inner.allows(),
            Parsed::V1_10_0(inner) => inner.allows(),
            Parsed::V1_11_0(inner) => inner.allows(),
            Parsed::V1_12_0(inner) => inner.allows(),
        }
    }

    fn semver_options(&self) -> &parsed::common::SemverOptions {
        match self {
            Parsed::V1(inner) => inner.semver_options(),
            Parsed::V1_10_0(inner) => inner.semver_options(),
            Parsed::V1_11_0(inner) => inner.semver_options(),
            Parsed::V1_12_0(inner) => inner.semver_options(),
        }
    }

    fn cuda_detection(&self) -> Option<bool> {
        match self {
            Parsed::V1(inner) => inner.cuda_detection(),
            Parsed::V1_10_0(inner) => inner.cuda_detection(),
            Parsed::V1_11_0(inner) => inner.cuda_detection(),
            Parsed::V1_12_0(inner) => inner.cuda_detection(),
        }
    }

    fn activate_mode(&self) -> Option<&ActivateMode> {
        match self {
            Parsed::V1(inner) => inner.activate_mode(),
            Parsed::V1_10_0(inner) => inner.activate_mode(),
            Parsed::V1_11_0(inner) => inner.activate_mode(),
            Parsed::V1_12_0(inner) => inner.activate_mode(),
        }
    }

    fn services_auto_start(&self) -> bool {
        match self {
            Parsed::V1(inner) => inner.services_auto_start(),
            Parsed::V1_10_0(inner) => inner.services_auto_start(),
            Parsed::V1_11_0(inner) => inner.services_auto_start(),
            Parsed::V1_12_0(inner) => inner.services_auto_start(),
        }
    }

    fn vars_mut(&mut self) -> &mut parsed::common::Vars {
        match self {
            Parsed::V1(inner) => inner.vars_mut(),
            Parsed::V1_10_0(inner) => inner.vars_mut(),
            Parsed::V1_11_0(inner) => inner.vars_mut(),
            Parsed::V1_12_0(inner) => inner.vars_mut(),
        }
    }

    fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook> {
        match self {
            Parsed::V1(inner) => inner.hook_mut(),
            Parsed::V1_10_0(inner) => inner.hook_mut(),
            Parsed::V1_11_0(inner) => inner.hook_mut(),
            Parsed::V1_12_0(inner) => inner.hook_mut(),
        }
    }

    fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile> {
        match self {
            Parsed::V1(inner) => inner.profile_mut(),
            Parsed::V1_10_0(inner) => inner.profile_mut(),
            Parsed::V1_11_0(inner) => inner.profile_mut(),
            Parsed::V1_12_0(inner) => inner.profile_mut(),
        }
    }

    fn services_mut(&mut self) -> &mut parsed::common::Services {
        match self {
            Parsed::V1(inner) => inner.services_mut(),
            Parsed::V1_10_0(inner) => inner.services_mut(),
            Parsed::V1_11_0(inner) => inner.services_mut(),
            Parsed::V1_12_0(inner) => inner.services_mut(),
        }
    }

    fn include_mut(&mut self) -> &mut parsed::common::Include {
        match self {
            Parsed::V1(inner) => inner.include_mut(),
            Parsed::V1_10_0(inner) => inner.include_mut(),
            Parsed::V1_11_0(inner) => inner.include_mut(),
            Parsed::V1_12_0(inner) => inner.include_mut(),
        }
    }

    fn build_mut(&mut self) -> &mut parsed::common::Build {
        match self {
            Parsed::V1(inner) => inner.build_mut(),
            Parsed::V1_10_0(inner) => inner.build_mut(),
            Parsed::V1_11_0(inner) => inner.build_mut(),
            Parsed::V1_12_0(inner) => inner.build_mut(),
        }
    }

    fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize> {
        match self {
            Parsed::V1(inner) => inner.containerize_mut(),
            Parsed::V1_10_0(inner) => inner.containerize_mut(),
            Parsed::V1_11_0(inner) => inner.containerize_mut(),
            Parsed::V1_12_0(inner) => inner.containerize_mut(),
        }
    }

    fn systems_mut(&mut self) -> &mut Option<Vec<System>> {
        match self {
            Parsed::V1(inner) => inner.systems_mut(),
            Parsed::V1_10_0(inner) => inner.systems_mut(),
            Parsed::V1_11_0(inner) => inner.systems_mut(),
            Parsed::V1_12_0(inner) => inner.systems_mut(),
        }
    }

    fn activate_mode_mut(&mut self) -> &mut Option<ActivateMode> {
        match self {
            Parsed::V1(inner) => inner.activate_mode_mut(),
            Parsed::V1_10_0(inner) => inner.activate_mode_mut(),
            Parsed::V1_11_0(inner) => inner.activate_mode_mut(),
            Parsed::V1_12_0(inner) => inner.activate_mode_mut(),
        }
    }
}

/// Forwarding impls for the `Manifest<S>` type-state wrappers. Each delegates
/// to the underlying `parsed` / `migrated_parsed` which is itself a `Parsed`.
macro_rules! impl_common_fields_for_manifest {
    ($state:ty, $field:ident) => {
        impl CommonFields for Manifest<$state> {
            fn vars(&self) -> &parsed::common::Vars {
                self.inner.$field.vars()
            }

            fn hook(&self) -> Option<&parsed::common::Hook> {
                self.inner.$field.hook()
            }

            fn profile(&self) -> Option<&parsed::common::Profile> {
                self.inner.$field.profile()
            }

            fn services(&self) -> &parsed::common::Services {
                self.inner.$field.services()
            }

            fn include(&self) -> &parsed::common::Include {
                self.inner.$field.include()
            }

            fn build(&self) -> &parsed::common::Build {
                self.inner.$field.build()
            }

            fn containerize(&self) -> Option<&parsed::common::Containerize> {
                self.inner.$field.containerize()
            }

            fn systems(&self) -> Option<&Vec<System>> {
                self.inner.$field.systems()
            }

            fn allows(&self) -> &parsed::common::Allows {
                self.inner.$field.allows()
            }

            fn semver_options(&self) -> &parsed::common::SemverOptions {
                self.inner.$field.semver_options()
            }

            fn cuda_detection(&self) -> Option<bool> {
                self.inner.$field.cuda_detection()
            }

            fn activate_mode(&self) -> Option<&ActivateMode> {
                self.inner.$field.activate_mode()
            }

            fn services_auto_start(&self) -> bool {
                self.inner.$field.services_auto_start()
            }

            fn vars_mut(&mut self) -> &mut parsed::common::Vars {
                self.inner.$field.vars_mut()
            }

            fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook> {
                self.inner.$field.hook_mut()
            }

            fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile> {
                self.inner.$field.profile_mut()
            }

            fn services_mut(&mut self) -> &mut parsed::common::Services {
                self.inner.$field.services_mut()
            }

            fn include_mut(&mut self) -> &mut parsed::common::Include {
                self.inner.$field.include_mut()
            }

            fn build_mut(&mut self) -> &mut parsed::common::Build {
                self.inner.$field.build_mut()
            }

            fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize> {
                self.inner.$field.containerize_mut()
            }

            fn systems_mut(&mut self) -> &mut Option<Vec<System>> {
                self.inner.$field.systems_mut()
            }

            fn activate_mode_mut(&mut self) -> &mut Option<ActivateMode> {
                self.inner.$field.activate_mode_mut()
            }
        }
    };
}

impl_common_fields_for_manifest!(Validated, parsed);
impl_common_fields_for_manifest!(TypedOnly, parsed);
impl_common_fields_for_manifest!(MigratedTypedOnly, migrated_parsed);
impl_common_fields_for_manifest!(Migrated, migrated_parsed);
