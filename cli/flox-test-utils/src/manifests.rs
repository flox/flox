//! Example manifests

/// An `[options]` table pinning the systems that the resolution recordings in
/// `test_data/generated` were made with (the old implicit default set).
///
/// Append this to fixture manifests that replay recordings so that their
/// resolve requests keep matching the recordings byte-for-byte on every build
/// host, now that the implicit default set includes the current system.
pub const ALL_SYSTEMS_OPTIONS: &str = r#"
[options]
systems = ["aarch64-darwin", "x86_64-darwin", "aarch64-linux", "x86_64-linux"]
"#;

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
