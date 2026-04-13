use crate::Parsed;
use crate::parsed::common;

/// CommonFields can be used inside the flox-manifest crate to access fields
/// that are the same across all schema versions
///
/// We don't want to use it outside the crate because we should be operating on
/// ManifestLatest outside the crate.
pub(crate) trait CommonFields {
    fn services(&self) -> &common::Services;
    #[cfg(test)]
    fn options_mut(&mut self) -> &mut common::Options;
}

impl CommonFields for Parsed {
    fn services(&self) -> &common::Services {
        match self {
            Parsed::V1(m) => &m.services,
            Parsed::V1_10_0(m) => &m.services,
            Parsed::V1_11_0(m) => &m.services,
        }
    }

    #[cfg(test)]
    fn options_mut(&mut self) -> &mut common::Options {
        match self {
            Parsed::V1(m) => &mut m.options,
            Parsed::V1_10_0(m) => &mut m.options,
            Parsed::V1_11_0(m) => &mut m.options,
        }
    }
}
