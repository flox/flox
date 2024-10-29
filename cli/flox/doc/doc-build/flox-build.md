---
title: FLOX-BUILD
section: 1
header: "Flox User Manuals"
...

```{.include}
./include/experimental-warning.md
```
> Feature flag: `build`

# NAME

flox-build - Build packages for Flox


# SYNOPSIS

```
flox [<general-options>] build
     [-d=<path>]
     [-L]
     [<package>]...
```

# DESCRIPTION

Build the specified `<package>` from the environment in `<path>`,
and output build artifacts at `result-<package>` adjacent to the environment.

## Manifest defined Packages

Possible values for `<package>` are all keys under the `build` attribute
in the `manifest.toml`.
If no `<package>` is specified, flox will attempt to build all packages
that are defined in the environment.

Packages are built by running the script defined in `build.<package>.command`
within a `bash` subshell.
The shell will behave as if `flox activate` was run
immediately prior to running the build script.

By default, builds are run in a sandbox.
For a sandboxed build, the current project is _copied_ into a sandbox directory.
To avoid copying excessive files, e.g. accumulating build artifacts from earlier
manual builds, `.env` files containing secrets etc.,
only files tracked by `git` are available.
Untracked files, files outside of the repository and notably network access
will be restricted to encourage "pure" and reproducible builds.
Builds can opt-out of the sandbox by setting `build.<package>.sandbox = "off"`.
With the sandbox disabled, building is equivalent
to running the build script manually within a shell created by `flox activate`.
Any build can access the _results_ of other builds
(including non-sandboxed ones) by referring to their name via `${<package>}`.
In the example below, the `app` package depends on the `dep` package
by using `${deps}/node_modules`.

`flox build` creates a temporary directory for the build script
to output build artifacts to.
The environment variable `out` is set to this directory,
and the build script is expected to copy or move artifacts to `$out`.

Upon conclusion of the build, the build result
will be symlinked to `result-<package>` adjacent to the `.flox` directory
that defines the package.


# OPTIONS

`-L`, `--build-logs`
:   Enable detailed logging emitted by the build scripts.
    **not implemented yet**

`<package>`
:   The package(s) to build.
    Possible values are all keys under the `build` attribute
    in the environment's `manifest.toml`.


```{.include}
./include/environment-options.md
./include/general-options.md
```

# EXAMPLES

`flox build` is an experimental feature.
To use it the `build` feature flag has to be enabled:

```shell
$ flox config --set-bool features.build true
# OR
$ export FLOX_FEATURE_BUILD=true
```

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
```

2. Build the pacakge and verify its contents:

```
$ flox build hello
$ ls ./result-hello
hello.txt
$ cat ./result-hello/hello.txt
hello, world
```

## Building a simple multi stage app

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

2. Install dependencies and add build instaructions

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

[`flox-activate(1)`](./flox-activate.md)
[`manifest.toml(5)`](./manifest.toml.md)
