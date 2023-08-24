# Flox CLI and Library

## Contents of the Repo

Currently this repo houses three rust crates:

- `flox`: the flox binary and reimplementation of the `bash` based flox.
- `flox-rust-sdk`: A library layer implementing flox's capabilities independent
  of the frontend.
- `floxd`: a potential flox daemon

## Development

```
$ flox develop .#rust-env
```

This sets up an environment with dependencies, rust toolchain, variable
and `pre-commit-hooks`.

In the environment, use [`cargo`](https://doc.rust-lang.org/cargo/)
to build the rust based cli.

- build and run flox
   ```
   $ cargo run -- <args>
   ```
- build a debug build of flox
   ```
   $ cargo build
   # builds to ./target/debug/flox
   ```
- build an optimized release build of flox
   ```
   $ cargo build --release
   # builds to ./target/release/flox
   ```

**Note:**

cargo based builds should only be used locally.
Flox must be buildable using `flox` or `nix`.

- format rust code:
  ```
  $ cargo fmt
  $ cargo fmt --check # just check
  ```
  The project is formatted using rustfmt and applies custom rules through
  `.rustfmt.toml`.
  A pre-commit hook is set up to check rust file formatting.
- format nix code
  ```
  $ alejandra .
  $ alejandra . --check # just check
  ```
  A pre-commit hook is set up to check nix file formatting.
- lint rust
  ```
  $ cargo clippy --all
  ```
- lint all files (including for formatting):
  ```
  $ pre-commit run -a
  ```

## Git

### CLA

- [ ] All commits in a Pull Request are [signed](https://docs.github.com/en/authentication/managing-commit-signature-verification/signing-commits) and Verified by Github or via GPG.
- [ ] As an outside contributor you need to accept the flox [Contributor License Agreement](.github/CLA.md) by adding your Git/Github details in a row at the end of the [`CONTRIBUTORS.csv`](.github/CONTRIBUTORS.csv) file by way of the same pull request or one done previously.

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

```
$ cz c
```

or

```
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
$ flox develop flox --command 'cargo test';
```

### Integration tests

Integration tests are written with `bats` and `expect`.
They are located in the `<flox>/tests` folder.
To run them:

```console
$ flox develop flox --command 'cargo build';
$ flox run '.#flox-tests' -- -- --flox ./target/debug/flox;
```
The first `--` separates the `flox run` command from any arguments you'd like to supply to `nix run`.
The second `--` separates the `nix run` arguments from arguments supplied to the `flox-tests` script defined in `pkgs/flox-tests/default.nix`.
The `--flox` flag specifies which `flox` executable to use as by default `flox` will be picked from the environment.
A third `--` can be used to pass arguments to `bats`.

**Important** the `flox-tests` option `--tests` must point to the `<flox>/tests/` directory
root which is used to locate various resources within test environments.

#### Continuous testing
When working on the test you would probably want to run them continuously on
every change. In that case run the following:

```console
$ flox develop flox --command 'cargo build';
$ flox run '.#flox-tests' -- -- --flox ./target/debug/flox --watch;
```

#### `bats` arguments
You can pass arbitrary flags through to `bats` using the third `--` separator - however
bugs in the `flox` CLI parser require you to use `sh -c` to wrap the command.
Failing to wrap will cause `flox` to "consume" the `--` rather than pass it
through to the inner command:

```console
$ flox develop flox --command 'cargo build';
$ flox run '.#flox-tests' -- -- \
  --flox ./target/debug/flox -- -j 4;
```
This example tells `bats` to run 4 jobs in parallel.

#### Running subsets of tests
You can specify which tests to run by passing arguments to either `flox-tests` or by directly passing arguments to `bats`.

In order to run a specific test file, pass the path to the file to `flox-tests`:
```console
$ flox run '.#flox-tests' -- -- \
--flox ./target/debug/flox ./tests/run.bats
```
This example will only run tests in the `tests/run.bats` file.

In order to run tests with a specific tag, you'll pass the `--filter-tags` option to `bats`:
```console
$ flox run '.#flox-tests' -- -- \
--flox ./target/debug/flox -- \
--filter-tags activate
```
This example will only run tests tagged with `activate`.
You can use boolean logic and specify the flag multiple times to run specific subsets of tests.
See the [bats usage documentation](https://bats-core.readthedocs.io/en/stable/usage.html) for details.
