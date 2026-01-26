pub(crate) trait InitManifest {
    pub fn new_documented(
        _features: Features,
        systems: &[&System],
        customization: &InitCustomization,
    ) -> toml_edit::DocumentMut;
    pub fn new_minimal(customization: &InitCustomization) -> toml_edit::DocumentMut;
}
