use crate::{Manifest, Migrated, MigratedTypedOnly, Parsed, TypedOnly, parsed};

pub trait CommonFields {
    fn vars(&self) -> &parsed::common::Vars;
    fn hook(&self) -> Option<&parsed::common::Hook>;
    fn profile(&self) -> Option<&parsed::common::Profile>;
    fn services(&self) -> &parsed::common::Services;
    fn include(&self) -> &parsed::common::Include;
    fn build(&self) -> &parsed::common::Build;
    fn containerize(&self) -> Option<&parsed::common::Containerize>;
    fn options(&self) -> &parsed::common::Options;

    fn vars_mut(&mut self) -> &mut parsed::common::Vars;
    fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook>;
    fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile>;
    fn services_mut(&mut self) -> &mut parsed::common::Services;
    fn include_mut(&mut self) -> &mut parsed::common::Include;
    fn build_mut(&mut self) -> &mut parsed::common::Build;
    fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize>;
    fn options_mut(&mut self) -> &mut parsed::common::Options;
}

impl CommonFields for Parsed {
    fn vars(&self) -> &parsed::common::Vars {
        match self {
            Parsed::V1(inner) => inner.vars(),
            Parsed::V1_10_0(inner) => inner.vars(),
        }
    }

    fn hook(&self) -> Option<&parsed::common::Hook> {
        match self {
            Parsed::V1(inner) => inner.hook(),
            Parsed::V1_10_0(inner) => inner.hook(),
        }
    }

    fn profile(&self) -> Option<&parsed::common::Profile> {
        match self {
            Parsed::V1(inner) => inner.profile(),
            Parsed::V1_10_0(inner) => inner.profile(),
        }
    }

    fn services(&self) -> &parsed::common::Services {
        match self {
            Parsed::V1(inner) => inner.services(),
            Parsed::V1_10_0(inner) => inner.services(),
        }
    }

    fn include(&self) -> &parsed::common::Include {
        match self {
            Parsed::V1(inner) => inner.include(),
            Parsed::V1_10_0(inner) => inner.include(),
        }
    }

    fn build(&self) -> &parsed::common::Build {
        match self {
            Parsed::V1(inner) => inner.build(),
            Parsed::V1_10_0(inner) => inner.build(),
        }
    }

    fn containerize(&self) -> Option<&parsed::common::Containerize> {
        match self {
            Parsed::V1(inner) => inner.containerize(),
            Parsed::V1_10_0(inner) => inner.containerize(),
        }
    }

    fn options(&self) -> &parsed::common::Options {
        match self {
            Parsed::V1(inner) => inner.options(),
            Parsed::V1_10_0(inner) => inner.options(),
        }
    }

    fn vars_mut(&mut self) -> &mut parsed::common::Vars {
        match self {
            Parsed::V1(inner) => inner.vars_mut(),
            Parsed::V1_10_0(inner) => inner.vars_mut(),
        }
    }

    fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook> {
        match self {
            Parsed::V1(inner) => inner.hook_mut(),
            Parsed::V1_10_0(inner) => inner.hook_mut(),
        }
    }

    fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile> {
        match self {
            Parsed::V1(inner) => inner.profile_mut(),
            Parsed::V1_10_0(inner) => inner.profile_mut(),
        }
    }

    fn services_mut(&mut self) -> &mut parsed::common::Services {
        match self {
            Parsed::V1(inner) => inner.services_mut(),
            Parsed::V1_10_0(inner) => inner.services_mut(),
        }
    }

    fn include_mut(&mut self) -> &mut parsed::common::Include {
        match self {
            Parsed::V1(inner) => inner.include_mut(),
            Parsed::V1_10_0(inner) => inner.include_mut(),
        }
    }

    fn build_mut(&mut self) -> &mut parsed::common::Build {
        match self {
            Parsed::V1(inner) => inner.build_mut(),
            Parsed::V1_10_0(inner) => inner.build_mut(),
        }
    }

    fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize> {
        match self {
            Parsed::V1(inner) => inner.containerize_mut(),
            Parsed::V1_10_0(inner) => inner.containerize_mut(),
        }
    }

    fn options_mut(&mut self) -> &mut parsed::common::Options {
        match self {
            Parsed::V1(inner) => inner.options_mut(),
            Parsed::V1_10_0(inner) => inner.options_mut(),
        }
    }
}

impl CommonFields for Manifest<TypedOnly> {
    fn vars(&self) -> &parsed::common::Vars {
        self.inner.parsed.vars()
    }

    fn hook(&self) -> Option<&parsed::common::Hook> {
        self.inner.parsed.hook()
    }

    fn profile(&self) -> Option<&parsed::common::Profile> {
        self.inner.parsed.profile()
    }

    fn services(&self) -> &parsed::common::Services {
        self.inner.parsed.services()
    }

    fn include(&self) -> &parsed::common::Include {
        self.inner.parsed.include()
    }

    fn build(&self) -> &parsed::common::Build {
        self.inner.parsed.build()
    }

    fn containerize(&self) -> Option<&parsed::common::Containerize> {
        self.inner.parsed.containerize()
    }

    fn options(&self) -> &parsed::common::Options {
        self.inner.parsed.options()
    }

    fn vars_mut(&mut self) -> &mut parsed::common::Vars {
        self.inner.parsed.vars_mut()
    }

    fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook> {
        self.inner.parsed.hook_mut()
    }

    fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile> {
        self.inner.parsed.profile_mut()
    }

    fn services_mut(&mut self) -> &mut parsed::common::Services {
        self.inner.parsed.services_mut()
    }

    fn include_mut(&mut self) -> &mut parsed::common::Include {
        self.inner.parsed.include_mut()
    }

    fn build_mut(&mut self) -> &mut parsed::common::Build {
        self.inner.parsed.build_mut()
    }

    fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize> {
        self.inner.parsed.containerize_mut()
    }

    fn options_mut(&mut self) -> &mut parsed::common::Options {
        self.inner.parsed.options_mut()
    }
}

impl CommonFields for Manifest<MigratedTypedOnly> {
    fn vars(&self) -> &parsed::common::Vars {
        self.inner.migrated_parsed.vars()
    }

    fn hook(&self) -> Option<&parsed::common::Hook> {
        self.inner.migrated_parsed.hook()
    }

    fn profile(&self) -> Option<&parsed::common::Profile> {
        self.inner.migrated_parsed.profile()
    }

    fn services(&self) -> &parsed::common::Services {
        self.inner.migrated_parsed.services()
    }

    fn include(&self) -> &parsed::common::Include {
        self.inner.migrated_parsed.include()
    }

    fn build(&self) -> &parsed::common::Build {
        self.inner.migrated_parsed.build()
    }

    fn containerize(&self) -> Option<&parsed::common::Containerize> {
        self.inner.migrated_parsed.containerize()
    }

    fn options(&self) -> &parsed::common::Options {
        self.inner.migrated_parsed.options()
    }

    fn vars_mut(&mut self) -> &mut parsed::common::Vars {
        self.inner.migrated_parsed.vars_mut()
    }

    fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook> {
        self.inner.migrated_parsed.hook_mut()
    }

    fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile> {
        self.inner.migrated_parsed.profile_mut()
    }

    fn services_mut(&mut self) -> &mut parsed::common::Services {
        self.inner.migrated_parsed.services_mut()
    }

    fn include_mut(&mut self) -> &mut parsed::common::Include {
        self.inner.migrated_parsed.include_mut()
    }

    fn build_mut(&mut self) -> &mut parsed::common::Build {
        self.inner.migrated_parsed.build_mut()
    }

    fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize> {
        self.inner.migrated_parsed.containerize_mut()
    }

    fn options_mut(&mut self) -> &mut parsed::common::Options {
        self.inner.migrated_parsed.options_mut()
    }
}

impl CommonFields for Manifest<Migrated> {
    fn vars(&self) -> &parsed::common::Vars {
        self.inner.migrated_parsed.vars()
    }

    fn hook(&self) -> Option<&parsed::common::Hook> {
        self.inner.migrated_parsed.hook()
    }

    fn profile(&self) -> Option<&parsed::common::Profile> {
        self.inner.migrated_parsed.profile()
    }

    fn services(&self) -> &parsed::common::Services {
        self.inner.migrated_parsed.services()
    }

    fn include(&self) -> &parsed::common::Include {
        self.inner.migrated_parsed.include()
    }

    fn build(&self) -> &parsed::common::Build {
        self.inner.migrated_parsed.build()
    }

    fn containerize(&self) -> Option<&parsed::common::Containerize> {
        self.inner.migrated_parsed.containerize()
    }

    fn options(&self) -> &parsed::common::Options {
        self.inner.migrated_parsed.options()
    }

    fn vars_mut(&mut self) -> &mut parsed::common::Vars {
        self.inner.migrated_parsed.vars_mut()
    }

    fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook> {
        self.inner.migrated_parsed.hook_mut()
    }

    fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile> {
        self.inner.migrated_parsed.profile_mut()
    }

    fn services_mut(&mut self) -> &mut parsed::common::Services {
        self.inner.migrated_parsed.services_mut()
    }

    fn include_mut(&mut self) -> &mut parsed::common::Include {
        self.inner.migrated_parsed.include_mut()
    }

    fn build_mut(&mut self) -> &mut parsed::common::Build {
        self.inner.migrated_parsed.build_mut()
    }

    fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize> {
        self.inner.migrated_parsed.containerize_mut()
    }

    fn options_mut(&mut self) -> &mut parsed::common::Options {
        self.inner.migrated_parsed.options_mut()
    }
}
