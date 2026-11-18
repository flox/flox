pub const HELLO: &str = r#"
    version = 1

    [install]
    hello.pkg-path = "hello"
"#;

/// A manifest with the `options.systems` that `flox init` wrote into every
/// manifest before x86_64-darwin was retired, which locking removes.
pub const FORMER_DEFAULT_SYSTEMS: &str = r#"
    version = 1

    [options]
    systems = ["aarch64-darwin", "aarch64-linux", "x86_64-darwin", "x86_64-linux"]
"#;
