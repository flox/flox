use flox_core::data::System;

use crate::Parsed;

/// CommonFields can be used inside the flox-manifest crate to access fields
/// that are the same across all schema versions
///
/// We don't want to use it outside the crate because we should be operating on
/// ManifestLatest outside the crate.
pub(crate) trait CommonFields {
    fn systems(&self) -> Option<&Vec<System>>;
    #[cfg(test)]
    fn systems_mut(&mut self) -> &mut Option<Vec<System>>;
}

impl CommonFields for Parsed {
    fn systems(&self) -> Option<&Vec<System>> {
        match self {
            Parsed::V1(m) => m.options.systems.as_ref(),
            Parsed::V1_10_0(m) => m.options.systems.as_ref(),
            Parsed::V1_11_0(m) => m.options.systems.as_ref(),
            Parsed::V1_12_0(m) => m.options.systems.as_ref(),
            Parsed::V1_13_0(m) => m.options.systems.as_ref(),
            Parsed::V1_14_0(m) => m.options.systems.as_ref(),
            Parsed::V1_15_0(m) => m.options.systems.as_ref(),
            Parsed::V1_16_0(m) => m.options.systems.as_ref(),
            Parsed::V1_17_0(m) => m.options.systems.as_ref(),
            Parsed::V1_18_0(m) => m.options.systems.as_ref(),
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
            Parsed::V1_14_0(m) => &mut m.options.systems,
            Parsed::V1_15_0(m) => &mut m.options.systems,
            Parsed::V1_16_0(m) => &mut m.options.systems,
            Parsed::V1_17_0(m) => &mut m.options.systems,
            Parsed::V1_18_0(m) => &mut m.options.systems,
        }
    }
}
