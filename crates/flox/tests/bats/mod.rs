use std::fs::{self, create_dir_all};
use std::os::unix;
use std::path::Path;
use std::process::{self, ExitCode};

use anyhow::Result;
use derive_more::{Deref, DerefMut};
use tempfile::TempDir;

/// **RUN WITH `cargo test -F bats-tests bats::`**
///
/// `-F bats-tests` includes the tests and `bats::` selects this test module
///
/// External bats tests (imported from flox-bash)
#[test]
fn bats_integration_environment() -> Result<ExitCode> {
    let mut test_command = bats_test("integration");

    Ok(ExitCode::from(
        test_command.status()?.code().expect("Expected ExitCode") as u8,
    ))
}

/// **RUN WITH `cargo test -F bats-tests bats::`**
///
/// `-F bats-tests` includes the tests and `bats::` selects this test module
///
/// External bats development tests (imported from flox-bash)
#[test]
fn bats_integration_development() -> Result<ExitCode> {
    let mut test_command = bats_test("package");
    Ok(ExitCode::from(
        test_command.status()?.code().expect("Expected ExitCode") as u8,
    ))
}

#[derive(Debug, Deref, DerefMut)]
struct Command(
    #[deref]
    #[deref_mut]
    process::Command,
    TempDir,
);

fn bats_test(test: &str) -> Command {
    let test_fake_root = create_test_root();
    let mut test_command = process::Command::new("bats");
    test_command.arg(test_fake_root.path().join(format!("tests/{test}.bats")));
    test_command.env("FLOX_CLI", env!("CARGO_BIN_EXE_flox"));
    // We need a store path to install to an environment.
    // In flox (sh) the test first builds a fresh `flox` package
    // and uses its store path.
    // Since `cargo test` will already build the flox binary
    // and rebuilding it in Nix takes a couple extra minutes,
    // we provide a store path provided by our devShell.
    test_command.env("FLOX_PACKAGE", env!("FLOX_SH_PATH"));
    test_command.env("FLOX_IMPLEMENTATION", "rust");
    // Disable metrics for all test invocations.
    test_command.env("FLOX_DISABLE_METRICS", "true");
    // Some externally called programs in the test may produce a warning
    // if LC_ALL is unset.
    // Since this is not the variable under test,
    // we set LC_ALL to a known default.
    test_command.env("LC_ALL", "C");
    test_command.current_dir(&test_fake_root);
    Command(test_command, test_fake_root)
}

/// The integration tests are expected to be run from the root of `flox-bash`.
/// When run from `flox(-rust)` we used to set the CWD to the source path of `flox-bash`.
/// However, this breaks the eval test (in particular [1]) which will fail inside a store path.
/// The solution here is to link `{flox-bash}/tests/* -> {tmp}/tests/`
/// and set the CWD to `{tmp}`.
/// Some tests [2] also write output to files in the same directory,
/// which fails if `tests` is a link to a store path.
/// thus _the contents_ are linked instead.
///
/// [1]: https://github.com/flox/flox-bash-private/blob/727a0b9c127ce990b8f18822c0a30957b017607d/tests/integration.bats#L125
/// [2]: https://github.com/flox/flox-bash-private/blob/727a0b9c127ce990b8f18822c0a30957b017607d/tests/integration.bats#L133
fn create_test_root() -> TempDir {
    let test_fake_root = TempDir::new().expect("Failed creating tests dir");
    let tests_path = test_fake_root.path().join("tests");
    create_dir_all(&tests_path).expect("Failed creating tests directory");

    for entry in fs::read_dir(Path::new(env!("FLOX_SH_FLAKE")).join("tests"))
        .unwrap()
        .flatten()
    {
        unix::fs::symlink(entry.path(), tests_path.join(entry.file_name())).unwrap()
    }
    test_fake_root
}
