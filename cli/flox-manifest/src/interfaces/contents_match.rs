use std::str::FromStr;

use toml_edit::DocumentMut;

use crate::{Manifest, Migrated, Validated};

pub trait ContentsMatch {
    fn contents_match(&self, contents: impl AsRef<str>) -> bool;
}

// NOTE: DocumentMut::to_string adds a trailing newline for certain TOML
//       items, so when comparing you need to normalize through DocumentMut,
//       otherwise you'll false negatives on matches.

impl ContentsMatch for Manifest<Validated> {
    fn contents_match(&self, contents: impl AsRef<str>) -> bool {
        let self_contents = self.inner.raw.to_string();
        let other_contents = DocumentMut::from_str(contents.as_ref());
        other_contents.is_ok_and(|contents| contents.to_string() == self_contents)
    }
}

impl ContentsMatch for Manifest<Migrated> {
    fn contents_match(&self, contents: impl AsRef<str>) -> bool {
        let self_contents = self.inner.migrated_raw.to_string();
        let other_contents = DocumentMut::from_str(contents.as_ref());
        other_contents.is_ok_and(|contents| contents.to_string() == self_contents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prove_that_documentmut_doesnt_round_trip() {
        let input = "version = 1";
        let doc = DocumentMut::from_str(input).unwrap();
        let output = doc.to_string();
        assert_ne!(output.as_str(), input);
    }
}
