# Flox Test Suite

This test suite is implemented using a framework `bats`, a `bash` like language
which transpiles to `bash` scripts.

Additionally certain tests use `expect` to test interactive usage of the CLI.

## Running the Test Suite

See [CONTRIBUTING](../../CONTRIBUTING.md).

## Test Suite Internals

### Setup and Teardown

`bats` recognizes a functions with reserved names to be run when initializing
and cleaning up after test runs.

- `setup_suite` and `teardown_suite`.
  + Run once for a single invocation of `bats` to setup/cleanup all test files.
  + These routines must be defined in the file
    [setup_suite.bash](./setup_suite.bash) which is a reserved filename
    recognized by `bats`.
  + This is the ideal place to define early environment setup such as
    environment variables, authorization tokens, etc.
  + Our `setup_suite` routines are responsible for recording the user's "real"
    environment variables and configs, and then creating a clean runtime
    environment which makes copies or these values in a disposable temporary
    directory prefix.
  + We use this routine to generate `ssh` keys, and launch an agent.
    - This agent is killed during `teardown_suite`.
  + We use this routine to establish a temporary `gitconfig` for both `global`
    and `system` under a temporary prefix.
    - This prevents tests which modify `gitconfig` from modifying a user's real
      configuration files, and prevents contamination between tests.
  + During `teardown_suite` we delete all `flox` environments with the prefix
    `_testing_*` using `flox delete NAME --force --origin`.
    + This is performed by the helper function `deleteAllTestEnvs`.
- `setup_file` and `teardown_file`
  + Run once at the beginning and end of each `NAME.bats` file.
  + Test files may provide their own definitions of these routines, but we
    recommend that you invoke the "common" helper routines `common_file_setup`
    and `common_file_teardown` ( these are the default routines ).
  + These generally create a temporary `FLOX_TEST_HOME` prefix with `XDG_*_HOME`
    and various other config setup.
    - This limits cross contamination between test files, and allows multiple
      test files to be run in parallel.
    - This behavior can be changed so that it is performed for each individual
      test in a file by invoking `common_file_setup test` in `setup_file` and
      adding `home_setup test` to `setup`.
      See [run.bats](./run.bats) for an example.
  + The variable `TEST_ENVIRONMENT` is set based on the current file's basename.
    For example `foo.bats` will yield `TEST_ENVIRONMENT=_testing_foo`.
    - This is performed by [setup_file_envname](./setup_suite.bash).
      + A similar helper `setup_test_envname` may be called in `setup` to
        generate a unique envname for individual tests.
        See [edit.bats](./edit.bats) for an example.
