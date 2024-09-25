---
title: FLOX-BUILD-CLEAN
section: 1
header: "Flox User Manuals"
...

```{.include}
./include/experimental-warning.md
```
> Feature flag: `build`

# NAME

flox-build-clean - Clean the build directory

# SYNOPSIS

```
flox [<general-options>] build clean
     [-d=<path>]
     [<package>]...
```

# DESCRIPTION

Remove the build artifacts for `<package>` from the environment in `<path>`.
Without `<package>` specified clean up all packages
and build related temporary data.


# OPTIONS

`<package>`
:   The package(s) to clean.
    Possible values are all keys under the `build` attribute
    in the environment's `manifest.toml`.
    If ommitted, will clean all build related data.


```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO

[`flox-build(1)`](./flox-build.md)
[`manifest.toml(5)`](./manifest.toml.md)
