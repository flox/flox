//! Best-effort probes of the host OS.

/// macOS product version (e.g. `15.5`) via [`sysinfo::System::os_version`]
/// — NOT the kernel release that the near-identically-named
/// `sys_info::os_release()` reports. Best-effort `None`.
pub(crate) fn macos_product_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    let raw = sysinfo::System::os_version();
    #[cfg(not(target_os = "macos"))]
    let raw = None;
    normalize_macos_product_version(raw)
}

/// A binary linked against a pre-macOS-11 SDK (or run under the system's
/// version-compat shim) reads back a constant `10.16`. No real macOS ever
/// reported that — Big Sur is 11.x — so treat it as unknown, not a version.
pub(crate) fn normalize_macos_product_version(raw: Option<String>) -> Option<String> {
    raw.filter(|version| version != "10.16" && !version.starts_with("10.16."))
}

/// Major component of the macOS product version, e.g. `26.0` -> `26`.
///
/// `None` off macOS, and whenever the version is unknown or unparseable.
/// Callers gating a feature on a minimum release must treat `None` as "older
/// than the minimum": an unreadable version is not evidence of a new host.
pub(crate) fn macos_major_version() -> Option<u32> {
    macos_product_version().as_deref().and_then(major_component)
}

fn major_component(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_macos_product_version_drops_compat_shim_sentinel() {
        // `10.16` is the version-compat shim's constant, never a real
        // product version — it must read as unknown, not ship on the wire.
        assert_eq!(
            normalize_macos_product_version(Some("10.16".to_string())),
            None
        );
        assert_eq!(
            normalize_macos_product_version(Some("10.16.0".to_string())),
            None
        );
        assert_eq!(
            normalize_macos_product_version(Some("15.5".to_string())).as_deref(),
            Some("15.5")
        );
        assert_eq!(normalize_macos_product_version(None), None);
    }

    #[test]
    fn major_component_parses_leading_number() {
        assert_eq!(major_component("26.0"), Some(26));
        assert_eq!(major_component("26"), Some(26));
        assert_eq!(major_component("15.5.1"), Some(15));
        assert_eq!(major_component(""), None);
        assert_eq!(major_component("not-a-version"), None);
    }
}
