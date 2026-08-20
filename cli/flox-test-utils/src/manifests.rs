/// An empty manifest that opts in to `x86_64-darwin`, which is no longer one of
/// the implicit default systems.
///
/// Use it with the recordings that were deliberately made for the full
/// four-system set (`resolve/bpftrace.yaml`, `resolve/darwin_ps_all.yaml`) to
/// cover resolution against explicitly requested non-default systems.
pub const EMPTY_ALL_SYSTEMS: &str = r#"
    version = 1

    [options]
    systems = ["aarch64-darwin", "x86_64-darwin", "aarch64-linux", "x86_64-linux"]
"#;

pub const HELLO: &str = r#"
    version = 1

    [install]
    hello.pkg-path = "hello"
"#;
