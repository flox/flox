use std::borrow::Cow;
use std::io::Stderr;
use std::sync::Mutex;

use anyhow::Context;
use once_cell::sync::Lazy;

pub mod colors;
mod completion;
pub mod dialog;
pub mod init;
pub mod logger;
pub mod metrics;

use regex::Regex;

static NIX_IDENTIFIER_SAFE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^[a-zA-Z0-9_-]+$"#).unwrap());
pub static TERMINAL_STDERR: Lazy<Mutex<Stderr>> = Lazy::new(|| Mutex::new(std::io::stderr()));

fn nix_str_safe(s: &str) -> Cow<str> {
    if NIX_IDENTIFIER_SAFE.is_match(s) {
        s.into()
    } else {
        format!("{s:?}").into()
    }
}

pub fn toml_to_json(toml_contents: &str) -> Result<serde_json::Value, anyhow::Error> {
    // This type annotation is needed otherwise it will try to convert it to ()
    let toml: toml::Value = toml::from_str(toml_contents).context("manifest was not valid TOML")?;
    let json = serde_json::to_value(toml).context("couldn't convert manifest to JSON")?;
    Ok(json)
}

#[cfg(test)]
mod test {
    use super::*;

    const TOML_CONTENTS: &str = r#"
    [my_table]
    foo = { bar = "baz" }

    [some_table]
    key = "value"
    "#;

    const JSON_CONTENTS: &str = r#"
    {
        "my_table": {
            "foo": {
                "bar": "baz"
            }
        },
        "some_table": {
            "key": "value"
        }
    }
    "#;

    #[test]
    fn converts_toml_to_json() {
        let json = toml_to_json(TOML_CONTENTS).unwrap();
        let expected_json: serde_json::Value = serde_json::from_str(JSON_CONTENTS).unwrap();
        assert_eq!(json, expected_json);
    }
}
