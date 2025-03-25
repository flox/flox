//! Example manifests

pub const EMPTY_ALL_SYSTEMS: &str = r#"
    version = 1

    [options]
    systems = ["aarch64-darwin", "x86_64-darwin", "aarch64-linux", "x86_64-linux"]
"#;

pub const HELLO: &str = r#"
    version = 1

    [install]
    hello.pkg-path = "hello"

    [options]
    systems = ["aarch64-darwin", "x86_64-darwin", "aarch64-linux", "x86_64-linux"]
"#;
