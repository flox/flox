use std::fmt::Display;
use std::path::Path;
use std::str::FromStr;

use derive_more::From;
use flox_types::catalog::System;
use flox_types::stability::Stability;
use runix::flake_ref::indirect::IndirectRef;
use runix::installable::{AttrPath, Attribute, FlakeAttribute, Installable, ParseInstallableError};
use runix::store_path::StorePath;
use thiserror::Error;

use super::environment::InstalledPackage;
use crate::prelude::ChannelRegistry;

#[derive(Debug, PartialEq, Eq, Clone, From)]
pub enum FloxPackage {
    Id(usize),
    StorePath(StorePath),
    Installable(Installable),
    Triple(FloxTriple),
}

impl FloxPackage {
    pub fn parse(
        package: &str,
        channels: &ChannelRegistry,
        default_channel: &str,
    ) -> Result<Self, ParseError> {
        // return if id
        if let Ok(id) = package.parse() {
            return Ok(Self::Id(id));
        }

        // return if store path
        if let Ok(path) = Path::new(package).canonicalize() {
            if let Ok(store_path) = StorePath::from_path(path) {
                return Ok(Self::StorePath(store_path));
            }
        }

        // return if looks like flake attribute
        if package.contains('#') {
            return Ok(Self::Installable(FlakeAttribute::from_str(package)?.into()));
        }

        // resolve triple
        Ok(Self::Triple(FloxTriple::parse(
            package,
            channels,
            default_channel,
        )?))
    }

    pub fn flox_nix_attribute(&self) -> Option<(Vec<String>, Option<String>)> {
        match self {
            FloxPackage::Id(_) => None,
            FloxPackage::StorePath(path) => Some(([path.to_string()].to_vec(), None)),
            FloxPackage::Installable(installable) => {
                Some(([installable.to_string()].to_vec(), None))
            },
            FloxPackage::Triple(FloxTriple {
                stability: _,
                channel,
                name,
                version,
            }) => {
                let attrpath = [channel.to_string()]
                    .into_iter()
                    .chain(name.iter().map(|i| i.as_ref().to_owned()))
                    .collect();

                Some((attrpath, version.to_owned()))
            },
        }
    }
}

impl From<InstalledPackage> for FloxPackage {
    fn from(value: InstalledPackage) -> Self {
        match value {
            InstalledPackage::Catalog(triple, _) => Self::Triple(triple),
            InstalledPackage::Installable(flake_attr, _) => {
                Self::Installable(Installable::FlakeAttribute(flake_attr))
            },
            InstalledPackage::StorePath(path) => Self::StorePath(path),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct FloxTriple {
    pub stability: Stability,
    pub channel: String,
    pub name: AttrPath,
    pub version: Option<String>,
}

impl FloxTriple {
    /// parses a triple or triple shorthand
    ///
    /// triple format:
    ///
    /// [[<stability>].[<channel>]].<attrpath>[@<version>]
    fn parse(
        package: &str,
        channels: &ChannelRegistry,
        default_channel: &str,
    ) -> Result<Self, ParseError> {
        let (package, version) = match package.rsplit_once('@') {
            Some((package, version)) => (package, Some(version)),
            None => (package, None),
        };

        // interpret an attribute as the key for a channel
        // and try to resolve it form the channel set
        let as_channel = |attr: &Attribute| -> Option<String> {
            channels.get_entry(attr.as_ref()).map(|_| attr.to_string())
        };

        // try to interpret an attribute as a stability
        let as_stability = |attr: &Attribute| -> Option<Stability> {
            let stability = attr.as_ref().parse().unwrap();
            if matches!(stability, Stability::Other(_)) {
                None
            } else {
                Some(stability)
            }
        };

        // FloxTriple constructor private to the parse() function
        // - insert default channel and/or stability
        // - extract version
        let new_triple =
            |stability: Option<Stability>, channel: Option<String>, attrpath: AttrPath| {
                FloxTriple {
                    stability: stability.unwrap_or_default(),
                    channel: channel.unwrap_or_else(|| default_channel.to_string()),
                    name: attrpath,
                    version: version.map(String::from),
                }
            };

        let attrpath = AttrPath::from_str(package)?;
        let mut components = attrpath.iter().peekable();
        let first = components.next().ok_or(ParseError::NoPackage)?;
        let second = components.next();

        let tail = |n| {
            let mut tail = attrpath.iter().skip(n).peekable();
            tail.peek().is_some().then(|| tail.collect())
        };

        // three or more components, where
        //
        // <stability>.<channel>.<attrpath...>
        if let (Some(stability), Some(channel), Some(tail)) =
            (as_stability(first), second.and_then(as_channel), tail(2))
        {
            return Ok(new_triple(Some(stability), Some(channel), tail));
        }

        // two or more components, where <channel> is ommitted
        // channels that are named after a stability, must use the first pattern
        //
        // <stability...>.<attr>
        if let (Some(stability), Some(tail)) = (as_stability(first), tail(1)) {
            return Ok(new_triple(Some(stability), None, tail));
        }

        // two or more components, where <stability> is ommitted
        //
        // <channel>.<attr...>
        if let (Some(channel), Some(tail)) = (as_channel(first), tail(1)) {
            return Ok(new_triple(None, Some(channel), tail));
        }

        // attrpath only, shorthand for
        //
        // <default stability>.<default channel>.<attrpath...>
        Ok(new_triple(None, None, attrpath.clone()))
    }

    pub fn into_installable(self, system: System) -> Installable {
        let flakeref = IndirectRef::new(self.channel, Default::default());
        let version_attr = self.version.map(|version| version.replace('.', "_"));

        let mut attrpath: Vec<String> = Vec::new();
        attrpath.extend(["evalCatalog".to_string(), system]);
        attrpath.extend(self.name.into_iter().map(|a| a.as_ref().into()));
        attrpath.extend(version_attr);

        let attrpath = attrpath.as_slice().try_into().unwrap();

        FlakeAttribute {
            flakeref: flakeref.into(),
            attr_path: attrpath,
            outputs: Default::default(),
        }
        .into()
    }
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Could not parse installable: {0}")]
    ParseInstallable(#[from] ParseInstallableError),

    #[error("No package specified")]
    NoPackage,
}

impl Display for FloxPackage {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use flox_types::constants::DEFAULT_CHANNEL;
    use once_cell::sync::Lazy;
    use runix::flake_ref::git::GitRef;

    use super::*;
    use crate::flox::FLOX_SH;
    use crate::prelude::Channel;

    static CHANNELS: Lazy<ChannelRegistry> = Lazy::new(|| {
        let mut channels = ChannelRegistry::default();
        channels.register_channel("flox", Channel::from_str("github:flox/floxpkgs").unwrap());
        channels
    });

    #[test]
    fn parse_fully_qualified() {
        let expected = FloxPackage::Triple(FloxTriple {
            stability: Stability::Unstable,
            channel: "flox".parse().unwrap(),
            name: "flox".parse().unwrap(),
            version: Some("0.0.4".to_string()),
        });

        let parsed = FloxPackage::parse("unstable.flox.flox@0.0.4", &CHANNELS, DEFAULT_CHANNEL)
            .expect("should parse");
        assert_eq!(parsed, expected);
    }

    #[test]
    fn parse_no_channel() {
        let expected = FloxPackage::Triple(FloxTriple {
            stability: Stability::Unstable,
            channel: "nixpkgs-flox".parse().unwrap(),
            name: "flox".parse().unwrap(),
            version: Some("0.0.4".to_string()),
        });

        let parsed = FloxPackage::parse("unstable.flox@0.0.4", &CHANNELS, DEFAULT_CHANNEL)
            .expect("should parse");
        assert_eq!(parsed, expected);
    }

    #[test]
    fn parse_no_stability_channel() {
        let expected = FloxPackage::Triple(FloxTriple {
            stability: Stability::default(),
            channel: "flox".parse().unwrap(),
            name: "flox".parse().unwrap(),
            version: Some("0.0.4".to_string()),
        });

        let parsed = FloxPackage::parse("flox.flox@0.0.4", &CHANNELS, DEFAULT_CHANNEL)
            .expect("should parse");
        assert_eq!(parsed, expected);
    }

    #[test]
    fn parse_nixpkgs() {
        let expected = FloxPackage::Triple(FloxTriple {
            stability: Stability::default(),
            channel: "nixpkgs-flox".parse().unwrap(),
            name: "flox".parse().unwrap(),
            version: Some("0.0.4".to_string()),
        });

        let parsed =
            FloxPackage::parse("flox@0.0.4", &CHANNELS, DEFAULT_CHANNEL).expect("should parse");
        assert_eq!(parsed, expected);
    }

    #[test]
    fn parse_store_path() {
        let expected = FloxPackage::StorePath(StorePath::from_str(FLOX_SH).unwrap());
        let parsed = FloxPackage::parse(FLOX_SH, &CHANNELS, DEFAULT_CHANNEL).expect("should parse");
        assert_eq!(parsed, expected);
    }

    #[test]
    fn parse_id() {
        let expected = FloxPackage::Id(2);
        let parsed = FloxPackage::parse("2", &CHANNELS, DEFAULT_CHANNEL).expect("should parse");
        assert_eq!(parsed, expected);
    }

    #[test]
    #[ignore = "In the nix sandbox the current directory is not a flake nor a repo due to file filters)"]
    fn parse_flakeref() {
        let expected = FloxPackage::Installable(
            FlakeAttribute {
                // during tests and build the current dir is set to the manifest dir
                flakeref: runix::flake_ref::FlakeRef::GitPath(GitRef {
                    url: url::Url::from_file_path(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .ancestors()
                            .nth(2)
                            .unwrap(),
                    )
                    .unwrap()
                    .try_into()
                    .unwrap(),
                    attributes: Default::default(),
                }),
                attr_path: ["packages", "aarch64-darwin", "flox"].try_into().unwrap(),
                outputs: Default::default(),
            }
            .into(),
        );
        let parsed =
            FloxPackage::parse(".#packages.aarch64-darwin.flox", &CHANNELS, DEFAULT_CHANNEL)
                .expect("should parse");
        assert_eq!(parsed, expected);
    }
}
