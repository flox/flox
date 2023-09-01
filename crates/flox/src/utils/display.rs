use flox_rust_sdk::prelude::flox_package::FloxPackage;
use itertools::Itertools;

pub fn packages_to_string(packages: &[FloxPackage]) -> String {
    match packages.len() {
        0 => "".to_string(),
        1 => format!("'{}'", packages[0]),
        2 => format!("'{}' and '{}'", packages[0], packages[1]),
        _ => {
            let almost_all = packages.len() - 1;
            format!(
                "'{}', and '{}'",
                packages.iter().take(almost_all).join("', '"),
                packages[almost_all],
            )
        },
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::prelude::flox_package::FloxTriple;
    use flox_types::stability::Stability;

    use super::*;

    /// Helper function to create a triple from a string
    pub fn create_triple(name: &str) -> FloxPackage {
        FloxPackage::Triple(FloxTriple {
            stability: Stability::Stable,
            channel: "nixpkgs-flox".parse().unwrap(),
            name: name.parse().unwrap(),
            version: None,
        })
    }

    #[test]
    fn test_packages_to_string() {
        let mut packages = vec![];
        assert_eq!(packages_to_string(&packages), "");
        packages.push(create_triple("hello"));
        assert_eq!(packages_to_string(&packages), "'hello'");
        packages.push(create_triple("curl"));
        assert_eq!(packages_to_string(&packages), "'hello' and 'curl'");
        packages.push(create_triple("ripgrep"));
        assert_eq!(
            packages_to_string(&packages),
            "'hello', 'curl', and 'ripgrep'"
        );
    }
}
