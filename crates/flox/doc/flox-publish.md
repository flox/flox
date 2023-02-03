---
title: FLOX-PUBLISH
section: 1
header: "flox User Manuals"
...

# NAME

flox-publish - build and publish project to flox channel

# SYNOPSIS

flox [ `<general-options>` ] publish [ `<options>` ]

# DESCRIPTION

Perform a build, (optionally) copy binaries to a cache,
and add package metadata to a flox channel.

# OPTIONS

```{.include}
./include/general-options.md
./include/development-options.md
```

## Publish Options

 `[ --build-repo <URL> ]`
:   The URL of the git repository from which to `flox build` the package.
    This is used both to build the package as it is being published
    and embedded in catalog metadata so that the package can be built
    from source if it cannot be fetched from a binary store.

    (Nix experts will recognize this repository as the source flake
    for the package.)

`[ --channel-repo <URL> ]`
:   The URL of the git channel repository to which package
    metadata should be published.
    See **subscribe** and **search** for descriptions on
    the use of channel repositories.

`[ --upload-to <URL> ]`
:   The URL of a binary cache location to which built package(s)
    should be copied.

`[ --download-from <URL> ]`
:   The URL from which built packages will be served at
    installation time.
    This URL typically refers to the same underlying resource
    as specified by the `--upload-to` argument, but using
    a different transport. For example, we upload packages
    to the (writable, authenticated) s3://flox-store-public URL,
    but users download these packages from the (read-only,
    unauthenticated) https://cache.floxdev.com endpoint.

    If not provided the `--download-from` argument will default to
    the same value as provided for the `--upload-to` argument.

`[ --render-path <dir> ]`
:   Sets the directory name for rendering the catalog
    within the git repository
    specified by the `--catalog-repo` flag.
    Defaults to "catalog" if not specified.

`[ --key-file <file> ]`
:   Used for identifying the path to the private key
    to be used in signing packages
    before analysis and upload.

When invoked without arguments, will prompt the user for the required values.

# USAGE OF PUBLISHED PACKAGES

Once published to a channel repository, you can then
search for and use your package with the following:

* subscribe to the channel: `flox subscribe <channel> <URL>`
* search for a package: `flox search -c <channel> <package>`
* install a package: `flox install <channel>.<package>`
