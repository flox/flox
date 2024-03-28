# Flox CLI and Library

## Quick Start
```console
$ cd git clone git@github.com/flox/flox.git;
$ cd flox;
# Enter Dev Shell
$ nix develop;
# Build `pkgdb' and `flox'
$ just build;
# Run the build
$ ./cli/target/debug/flox --help;
# Run the test suite
$ just test-all;
```

## PR Guidelines

### CLA

- [ ] All commits in a Pull Request are
      [signed](https://docs.github.com/en/authentication/managing-commit-signature-verification/signing-commits)
      and Verified by Github or via GPG.
- [ ] As an outside contributor you need to accept the flox
      [Contributor License Agreement](.github/CLA.md) by adding your Git/Github
      details in a row at the end of the
      [`CONTRIBUTORS.csv`](.github/CONTRIBUTORS.csv) file by way of the same
      pull request or one done previously.

### CI

CI can only be run against the flox/flox repository - it can't be run on forks.
To run CI on external contributions, a maintainer will have to fetch the branch
for a PR and push it to a branch in the flox/flox repo.
The maintainer reviewing a PR will run CI after approving the PR.
If you ever need a run triggered, feel free to ping your reviewer!

### Commits

This project follows (tries to),
[conventional commits](https://www.conventionalcommits.org/en/v1.0.0/).

We employ [commitizen](https://commitizen-tools.github.io/commitizen/)
to help enforcing those rules.

**Commit messages that explain the content of the commit are appreciated**

-----

For starters: commit messages shold have to follow the pattern:

```
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

The commit contains the following structural elements,
to communicate intent to the consumers of your library:

1. **fix**: a commit of the type `fix` patches a bug in your codebase
   (this correlates with PATCH in Semantic Versioning).
2. **feat**: a commit of the type feat introduces a new feature to the codebase
   (this correlates with MINOR in Semantic Versioning).
3. **BREAKING CHANGE**: a commit that has a footer BREAKING CHANGE:,
   or appends a ! after the type/scope, introduces a breaking API change
   (correlating with MAJOR in Semantic Versioning).
   A BREAKING CHANGE can be part of commits of any type.
4. types other than fix: and feat: are allowed,
   for example @commitlint/config-conventional (based on the Angular convention)
   recommends `build`, `chore`, `ci`, `docs`, `style`, `refactor`, `perf`,
   `test`, and others.
5. footers other than BREAKING CHANGE: <description> may be provided
   and follow a convention similar to git trailer format.

Additional types are not mandated by the Conventional Commits specification,
and have no implicit effect in Semantic Versioning
(unless they include a BREAKING CHANGE).

A scope may be provided to a commit’s type,
to provide additional contextual information
and is contained within parenthesis, e.g., feat(parser): add ability to parse
arrays.

-----

A pre-commit hook will ensure only correctly formatted commit messages are
committed.

You can also run

```console
$ cz c
```

or

```console
$ cz commit
```

to make conforming commits interactively.

## Contents of the Repo

Currently this repo houses three rust crates:

- `flox`: the flox binary and reimplementation of the `bash` based flox.
- `flox-rust-sdk`: A library layer implementing flox's capabilities independent
  of the frontend.
- `floxd`: a potential flox daemon

## Development

```console
$ nix develop;
```

This sets up an environment with dependencies, rust toolchain, variable
and `pre-commit-hooks`.

In the environment, use [`cargo`](https://doc.rust-lang.org/cargo/)
to build the rust based cli.

**Note:**

cargo based builds should only be used locally.
Flox must be buildable using `flox` or `nix`.

### Build and Run `flox`

- build and run flox
   ```console
   $ pushd cli;
   $ cargo run -- <args>;
   ```
- build a debug build of flox
   ```console
   $ just build;
   # builds to ./cli/target/debug/flox
   ```
- run flox unit tests
   ```console
   $ pushd cli;
   $ cargo test [<args>]
   # or enable impure and other long running tests with
   $ cargo -F extra-tests [<args>]
   ```
- build an optimized release build of flox
   ```console
   $ make -C pkgdb -j RELEASE=1;
   $ ( pushd cli||return; cargo build --release; )
   # builds to ./cli/target/release/flox
   ```

### Lint and Format `flox`

- format rust code:
  ```console
  $ pushd cli;
  $ cargo fmt
  $ cargo fmt --check # just check
  ```
  The project is formatted using rustfmt and applies custom rules through
  `.rustfmt.toml`.
  A pre-commit hook is set up to check rust file formatting.
- format nix code
  ```console
  $ alejandra .
  $ alejandra . --check # just check
  ```
  A pre-commit hook is set up to check nix file formatting.
- lint rust
  ```console
  $ pushd cli;
  $ cargo clippy --all
  ```
- lint all files (including for formatting):
  ```console
  $ pre-commit run -a
  ```

### Setting up C/C++ IDE support

For VSCode, it's suggested to use the *C/C++* plugin from *Microsoft*.  This
section refers to settings from that plugin.

- Set your environment to use `c17` and `c++20` standards.
  - For VSCode, this is available under *C++ Configuration*
- *Compile commands* are built with `just build-cdb` and will be placed in the
  root folder as `compile_commands.json`.
  - For VSCode, this is available under the *Advanced* drop down under
  *C++ Configuration* and can be set to `${workspaceFolder}/compile_commands.json`.


### Setting up rust IDE support

- Install the `rust-analyzer` plugin for your editor
   - See the [official installation instruction](https://rust-analyzer.github.io/manual.html#installation)
     (in the `nix develop` subshell the toolchain will aready be provided, you
      can skip right to your editor of choice)
- If you prefer to open your editor at the project root, you'll need to help
  `rust-analyzer` find the rust workspace by configuing the`linkedProjects`
  for `rust-analyzer`.
  In VS Code you can add this: to you `.vscode/settings.json`:
  ```json
  "rust-analyzer.linkedProjects": [
     "${workspaceFolder}/cli/Cargo.toml"
  ]
  ```
- If you want to be able to run and get analytics on impure tests, you need to
  activate the `extra-tests` feature
  In VS Code you can add this: to you `.vscode/settings.json`:
  ```json
  "rust-analyzer.cargo.features": [
     "extra-tests"
  ]
  ```
- If you use `foxundermoon.shell-format` on VS Code make sure to configure editor config support for it:
  ```
   "shellformat.useEditorConfig": true,
  ```

## Testing

### Unit tests

Unit test are ran with `cargo`.
These cover code authored in Rust, but does not explicitly cover code authored
in `<flox>/flox-bash/`.

```console
$ nix develop --command 'just test-all';
```

### Integration tests

Integration tests are written with `bats` and `expect`.
They are located in the `<flox>/tests` folder.
To run them:

```console
$ nix develop --command 'just build';
$ nix develop --command 'just integ-tests';
```

**Important** the `flox-tests` option `--tests` must point to the
`<flox>/tests/` directory root which is used to locate various resources within
test environments.

#### Continuous testing
When working on the test you would probably want to run them continuously on
every change. In that case run the following:

```console
$ nix develop --command 'just build';
$ nix develop --command '
    flox-tests --pkgdb "$PWD/pkgdb/bin/pkgdb"       \
               --flox "$PWD/cli/target/debug/flox"  \
               --watch;
  ';
```

#### `bats` arguments
You can pass arbitrary flags through to `bats` using a `--` separator.

```console
$ nix develop --command 'just build';
$ nix develop --command 'flox-tests --flox "$PWD/cli/target/debug/flox" -- -j 4';
```
This example tells `bats` to run 4 jobs in parallel.

#### Running subsets of tests
You can specify which tests to run by passing arguments to either `flox-tests`
or by directly passing arguments to `bats`.

##### Running a specific file
In order to run a specific test file, pass the path to the file to `flox-tests`:
```console
$ flox-tests --flox ./cli/target/debug/flox ./tests/run.bats;
$ or, using the Justfile
$ just bats-file ./tests/run.bats
```
This example will only run tests in the `tests/run.bats` file.


##### Running tagged tests
When writing integration tests it's important to add tags to each test to
identify which subsystems the integration test is using.
This makes it easier to target a test run at the subsystem you're working on.

You add tags to a test with a special comment:
```
# bats test_tags=foo,bar,baz
@test "this is the name of my test" {
   run "$FLOX_BIN" --help;
   assert_success;
}
```

You can apply a tag to tests in a file with another special comment, which
applies the tags to all of the tests that come after the comment:
```
# bats file_tags=foo

@test "this is the name of my test" {
   run "$FLOX_BIN" --help;
   assert_success;
}


@test "this is the name of my test" {
   run "$FLOX_BIN" --help;
   assert_success;
}
```

Tags cannot contain whitespace, but may contain `-`, `_`, and `:`, where `:` is
used for namespacing.

The list of tags to use for integration tests is as follows:
- `init`
- `build_env`
- `install`
- `uninstall`
- `activate`
- `push`
- `pull`
- `search`
- `edit`
- `list`
- `delete`
- `upgrade`
- `project_env`
- `managed_env`
- `remote_env`
- `python`, `node`, `go`, `ruby`, etc (anything language specific)

Some of these tags will overlap. For example, the `build_env` tag should be used
any time an environment is built, so there is overlap with `install`,
`activate`, etc.

In order to run tests with a specific tag, you'll pass the `--filter-tags`
option to `bats`:
```console
$ flox-tests --flox ./cli/target/debug/flox  \
             -- --filter-tags activate;
$ # or, using the Justfile
$ just bats-tests --filter-tags activate
```
This example will only run tests tagged with `activate`.
You can use boolean logic and specify the flag multiple times to run specific
subsets of tests.
See the [bats usage documentation](https://bats-core.readthedocs.io/en/stable/usage.html)
for details.

## Merges

Changes should be **squashed and merged** into `main`.

Development is done on branches and merged back to `main`.  Keep your branch
updated by rebasing and resolving conflicts regularly to avoid messy merges.
Merges to `main` should be squashed and *ff-only* back to `main` using GitHub
PRs.  Or, if they represent multiple bigger changes, squashed into multiple
distinct change sets.  Also be sure to run all tests before creating a mergable
PR (See [below](#testing)).
