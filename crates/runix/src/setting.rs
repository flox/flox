use std::fmt::Display;

use derive_more::From;

/// Setting Container akin to https://cs.github.com/NixOS/nix/blob/499e99d099ec513478a2d3120b2af3a16d9ae49d/src/libutil/config.cc#L199
#[derive(From, Clone)]
pub struct Setting<T>(T);

impl std::fmt::Display for Setting<Vec<String>> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.join(" "))
    }
}

impl<T> Setting<T>
where
    Setting<T>: Display,
{
    pub fn to_args(&self, flag: impl Into<String>) -> Vec<String> {
        vec![flag.into(), format!("{self}")]
    }
}

impl Setting<bool> {
    pub fn to_args(&self, flag: impl Into<String>) -> Vec<String> {
        vec![flag.into()]
    }
}
