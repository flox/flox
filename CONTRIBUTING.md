# Flox CLI and Library

## Quick Start

```console
# Clone the repository
$ cd git clone git@github.com/flox/flox.git
$ cd flox

# Enter development environment
$ nix develop

# Build `pkgdb' and `flox'
$ just build-pkg

# Run pkgdb
$ ./pkgdb/build/bin/pkgdb --help

# Run flox
$ ./cli/target/debug/flox --help

# Run the all tests
$ just test-all;
```

## Contents of the Repo

- `./pkgdb`: CRUD operations on `nix` package metadata.
- `./cli`: A flox CLI.
    - `./cli/flox`: the flox binary.
    - `./cli/flox-rust-sdk`: A library layer implementing capabilities
                             independent of the CLI.
    - `./cli/floxd`: a potential flox daemon (TODO)
- `./end2end`: A suite of functional tests for our flox CLI.
- `./docs`: Developer documentation.
- `./pkgs`: A nix package set for our projects.
- `./shells`: A development environment.
- `./flake.{nix|lock}`: Two Nix files that hold everything together.


## Development

1. [Install Nix](https://nixos.org/download.html).

2. Enable flakes by adding the following to `/etc/nix/nix.conf`:

```nix
experimental-features = nix-command flakes
```

3. Enter development environment:

```console
$ nix develop
```

### Develop `flox-cli`

| Description | Command                                     |
| :---------: | ------------------------------------------- |
| Build       | `just build-cli`                            |
| Test        | `just test-cli`                             |
| Run         | `just run-cli` or `./cli/target/debug/flox` |
| Format      | `just fmt-cli`                              |
| Clean       | `just clean-cli`                            |

The `cli` project is formatted using `rustfmt` and applies custom rules through
`./cli/.rustfmt.toml`. A pre-commit hook is set up as well to check rust file
formatting.


### Develop `flox-pkgdb`

| Description | Command                                       |
| :---------: | --------------------------------------------- |
| Build       | `just build-pkgdb`                            |
| Test        | `just test-pkgdb`                             |
| Run         | `just run-pkgdb` or `./pkgdb/build/bin/pkgdb` |
| Format      | `just fmt-pkgdb`                              |
| Clean       | `just clean-pkgdb`                            |


### Develop `flox-end2end`

| Description | Command                                       |
| :---------: | --------------------------------------------- |
| Test        | `just test-pkgdb`                             |
| Run         | `just run-pkgdb`                              |
| Format      | `just fmt-end2end`                            |
| Clean       | `just clean-end2end`                          |


How to use pytest at its best:

- You can run tests also directly with `pytest` script which should be
  available in the development environment.

- Test to be picked up by the `pytest` framework need to start with `test_`
  prefix. See example: `end2end/test_python.py`

- [Test fixtures](https://docs.pytest.org/en/stable/explanation/fixtures.html)
  are defned in `end2end/conftest.py`.

- When you increase verbosity, by providing `-v`, `pexpect`'s buffer is going
  to be printed to `stdout`, eg. `just test-end2end -v`

- If you wish to be dropped into a debugger when test fails run it with
  `--pdb`, eg: `just test-end2end --pdb`.

- To set a breakpoint manually drop `__import__('pdb').set_trace()` onto the
  line you wish to break.

- To run tests as CI would do it, run: `nix run .#flox-end2end`.


## Editors

### Setup VSCode for Rust development

1. `rust-analyzer` will be already installed in the development environment
   once you run `nix develop`.

2. If you prefer to open your editor at the project root, you'll need to help
   `rust-analyzer` find the rust workspace by configuing the`linkedProjects`
   for `rust-analyzer`.

   In VSCode you can add this: to you `.vscode/settings.json`:

   ```json
   "rust-analyzer.linkedProjects": [
      "${workspaceFolder}/cli/Cargo.toml"
   ]
   ```

3. If you want to be able to run and get analytics on impure tests, you need to
   activate the `extra-tests` feature.

   In VSCode you can add this: to you `.vscode/settings.json`:
   ```json
   "rust-analyzer.cargo.features": [
      "extra-tests"
   ]
   ```

## Git

### CLA

- [ ] All commits in a Pull Request are
      [signed](https://docs.github.com/en/authentication/managing-commit-signature-verification/signing-commits)
      and Verified by Github or via GPG.
- [ ] As an outside contributor you need to accept the flox 
      [Contributor License Agreement](.github/CLA.md) by adding your Git/Github 
      details in a row at the end of the 
      [`CONTRIBUTORS.csv`](.github/CONTRIBUTORS.csv) file by way of the same 
      pull request or one done previously.

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

### Merges

This repo follows a variant of [git-flow](https://www.atlassian.com/git/tutorials/comparing-workflows/gitflow-workflow).

Features are branched off the `develop` branch and committed back to it,
upon completion, using GitHub PRs.
Features should be **squashed and merged** into `develop`,
or if they represent multiple bigger changes,
squashed into multiple distinct change sets.

### Releases

**TODO**



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
