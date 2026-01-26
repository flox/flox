use crate::{Manifest, Migrated, Validated};

pub trait ContentsMatch {
    fn contents_match(&self, contents: impl AsRef<str>) -> bool;
}

impl ContentsMatch for Manifest<Validated> {
    fn contents_match(&self, contents: impl AsRef<str>) -> bool {
        let self_contents = self.inner.raw.to_string();
        self_contents.as_str() == contents.as_ref()
    }
}

impl ContentsMatch for Manifest<Migrated> {
    fn contents_match(&self, contents: impl AsRef<str>) -> bool {
        let self_contents = self.inner.migrated_raw.to_string();
        self_contents.as_str() == contents.as_ref()
    }
}
