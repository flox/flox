use flox_core::data::System;

use crate::Parsed;
use crate::parsed::common;

/// CommonFields can be used inside the flox-manifest crate to access fields
/// that are the same across all schema versions
///
/// We don't want to use it outside the crate because we should be operating on
/// ManifestLatest outside the crate.
///
/// V1_13_0 has a version-specific `Options` (with `sandbox`), so the options
/// accessor exposes only the `systems` field shared by every version rather
/// than a uniform `&common::Options`.
pub(crate) trait CommonFields {
    fn services(&self) -> &common::Services;
    fn systems(&self) -> Option<&Vec<System>>;
    #[cfg(test)]
    fn systems_mut(&mut self) -> &mut Option<Vec<System>>;
}

impl CommonFields for Parsed {
    fn services(&self) -> &common::Services {
        match self {
            Parsed::V1(m) => &m.services,
            Parsed::V1_10_0(m) => &m.services,
            Parsed::V1_11_0(m) => &m.services,
            Parsed::V1_12_0(m) => &m.services.service_map,
            Parsed::V1_13_0(m) => &m.services.service_map,
        }
    }

    fn systems(&self) -> Option<&Vec<System>> {
        match self {
            Parsed::V1(m) => m.options.systems.as_ref(),
            Parsed::V1_10_0(m) => m.options.systems.as_ref(),
            Parsed::V1_11_0(m) => m.options.systems.as_ref(),
            Parsed::V1_12_0(m) => m.options.systems.as_ref(),
            Parsed::V1_13_0(m) => m.options.systems.as_ref(),
        }
    }

    #[cfg(test)]
    fn systems_mut(&mut self) -> &mut Option<Vec<System>> {
        match self {
            Parsed::V1(m) => &mut m.options.systems,
            Parsed::V1_10_0(m) => &mut m.options.systems,
            Parsed::V1_11_0(m) => &mut m.options.systems,
            Parsed::V1_12_0(m) => &mut m.options.systems,
            Parsed::V1_13_0(m) => &mut m.options.systems,
        }
    }
}
