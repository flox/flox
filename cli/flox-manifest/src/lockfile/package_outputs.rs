use std::collections::BTreeMap;

use super::{LockedPackageCatalog, LockedPackageFlake};

/// A trait for listing the outputs of packages.
///
/// This helps paper over the fact that outputs are stored
/// in different ways for different locked package types.
///
/// This is implemented as a trait rather than a method on [`LockedPackage`]
/// because we don't list outputs for [`LockedPackageStorePath`].
pub trait PackageOutputs {
    fn outputs(&self) -> BTreeMap<String, String>;
    fn outputs_to_install(&self) -> Option<Vec<String>>;
    /// Returns the list of all outputs for the package.
    fn all_outputs(&self) -> Vec<String> {
        self.outputs().keys().cloned().collect::<Vec<_>>()
    }
    /// Returns the deduplicated list of outputs to install.
    ///
    /// Note that this assumes the particular behavior of the catalog-server
    /// bug that causes the duplication in the first place, which is that you
    /// end up with runs of repeated outputs (`"out"` only, as far as I can tell).
    fn deduped_outputs_to_install(&self) -> Option<Vec<String>> {
        self.outputs_to_install().map(|output_list| {
            let mut to_dedup = output_list.clone();
            to_dedup.dedup();
            to_dedup
        })
    }
    /// Returns `true` if `outputs_to_install` exists and matches the
    /// full list of outputs.
    fn outputs_match_outputs_to_install(&self) -> Option<bool> {
        self.deduped_outputs_to_install().map(|outputs_to_install| {
            let mut sorted_oti = outputs_to_install.clone();
            sorted_oti.sort();
            let mut sorted_all_outputs = self.all_outputs();
            sorted_all_outputs.sort();
            sorted_oti == sorted_all_outputs
        })
    }
}

impl PackageOutputs for LockedPackageCatalog {
    fn outputs(&self) -> BTreeMap<String, String> {
        self.outputs.clone()
    }

    fn outputs_to_install(&self) -> Option<Vec<String>> {
        self.outputs_to_install.clone()
    }
}

impl PackageOutputs for LockedPackageFlake {
    fn outputs(&self) -> BTreeMap<String, String> {
        self.locked_installable.outputs.clone()
    }

    fn outputs_to_install(&self) -> Option<Vec<String>> {
        self.locked_installable.outputs_to_install.clone()
    }
}
