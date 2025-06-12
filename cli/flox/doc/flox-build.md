---
title: FLOX-BUILD
section: 1
header: "Flox User Manuals"
...


# NAME

flox-build - Build packages for Flox


# SYNOPSIS

```
flox [<general-options>] build
     [-d=<path>]
     [<package>]...
```

# DESCRIPTION

Build the specified `<package>` from the environment in `<path>`,
and output build artifacts at `result-<package>` adjacent to the environment.

## Manifest-defined artifacts

Possible values for `<package>` are all keys under the `build` attribute
in the `manifest.toml`.
If no `<package>` is specified, Flox will attempt to build all packages
that are defined in the `manifest.toml`.

Packages are built by running the script defined in `build.<package>.command`
within a `bash` subshell.
The shell will behave as if `flox activate` was run immediately prior to
running the build script.

Builds are always run in a temporary directory into which all `git` tracked
files are copied.
A sandbox can be optionally enabled by setting
`build.<package>.sandbox = "pure"`.
For this kind of "sandboxed" build, access to untracked files, files outside of
the repository, and the network are restricted to provide a reproducible build
environment.
With the sandbox disabled, building is similar to running the build script
manually within a shell created by `flox activate`.

Any build can access the _results_ of other builds (including non-sandboxed
ones) by referring to their name via `${<package>}`.
This allows multi-stage builds.
In the example below, the `app` package depends on the `dep` package
by using `${deps}/node_modules`.

`flox build` creates a temporary directory for the build script
to output build artifacts to.
The environment variable `out` is set to this directory,
and the build script is expected to copy or move artifacts to `$out`.

Upon completion of the build, the build result will be symlinked to
`result-<package>` adjacent to the `.flox` directory that defines the package.

### Metadata

Specifying the `build.<package>.description>` and `build.<package>.version`
fields of the build provide extra metadata that can be used by `flox install`,
`flox search`, and `flox show` commands if the build is later published.

# OPTIONS

`<package>`
:   The package(s) to build.
    Possible values are all keys under the `build` attribute
    in the environment's `manifest.toml`.


```{.include}
./include/dir-environment-options.md
./include/general-options.md
```

# EXAMPLES

## Building a simple pure package

1. Add build instructions to the manifest:

```toml
# file: .flox/env/manifest.toml

...
[build]
hello.command = '''
# produce something and move it to $out
mkdir -p $out
echo "hello world" >> $out/hello.txt
'''
description = "Produces a file containing 'hello world'"
version = "0.0.0"
```

2. Build the package and verify its contents:

```
$ flox build hello
$ ls ./result-hello
hello.txt
$ cat ./result-hello/hello.txt
hello, world
```

## Building a simple multi-stage app

Assume a simple `nodejs` project

```
.
├── .git/
├── package-lock.json
├── package.json
├── public/
├── README.md
├── src/
...
```

1. Initialize a Flox environment

```shell
$ flox init
```

2. Install dependencies and add build instructions

```toml
# file: .flox/env/manifest.toml
version = 1

[install]
nodejs.pkg-path = "nodejs"
rsync.pkg-path = "rsync"

# install node dependencies using npm
# disable the sandbox to allow access to the network
[build]
deps.command = '''
npm ci
mkdir -p $out
mv node_modules $out/node_modules
'''
deps.sandbox = "off"

# build the application using previously fetched dependencies
app.command = '''
rsync -lr ${deps}/node_modules ./
npm run build
mv dist $out/
'''
```

3. Verify the result

```shell
$ npx serve result-app
```

# SEE ALSO

[`flox-build-clean(1)`](./flox-build-clean.md)
[`flox-activate(1)`](./flox-activate.md)
[`manifest.toml(5)`](./manifest.toml.md)
