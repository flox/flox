use crate::{Manifest, Migrated, Validated};

pub trait ContentsMatch {
    fn contents_match(&self, contents: impl AsRef<str>) -> bool;
}

impl ContentsMatch for Manifest<Validated> {
    fn contents_match(&self, contents: impl AsRef<str>) -> bool {
        let self_contents = self.inner.raw.to_string();
        // toml_edit's DocumentMut::to_string() may add a trailing newline
        // that the input string doesn't have (or vice versa), so we trim
        // trailing whitespace from both sides before comparing.
        self_contents.trim_end() == contents.as_ref().trim_end()
    }
}

impl ContentsMatch for Manifest<Migrated> {
    fn contents_match(&self, contents: impl AsRef<str>) -> bool {
        let self_contents = self.inner.migrated_raw.to_string();
        // See the Validated impl for reasoning.
        self_contents.trim_end() == contents.as_ref().trim_end()
    }
}
