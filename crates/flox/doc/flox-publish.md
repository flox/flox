---
title: FLOX-PUBLISH
section: 1
header: "flox User Manuals"
...

# NAME

flox-publish - build and publish project to flox channel

# SYNOPSIS

flox [ `<general-options>` ] publish [ `<options>` ] [`<package>`]

# DESCRIPTION

Adds metadata to a catalog for the package identified by `<package>`
or dynamically selected from a choice of packages in the current directory.

Prior to to submitting the metadata,
it will verify that the package can be built from its _upstream_ location
and optionally sign and cache the resulting binary.

The package must be defined in a remote git repository and be referred to
either directly by a `git+ssh://<url>[#<package>]` url or another url that
can be resolved to an upstream git resource.
Packages referred to by a `github:<user>/<owner>[#<package>]` URL are
resolved to `ssh://git@github.com` by default or `https://github.com`,
if `--prefer-https` is provided.
Packages in a local repository will be built from the upstream branch.
The local branch must be clean (i.e. have no uncommited changes) and
must be at the same revision as its upstream.

The metadata will be published in a `catalog/<system>` branch on the
upstream repository.


# OPTIONS

```{.include}
./include/general-options.md
./include/development-options.md
```

## Publish Options

`[ --cache-url <URL> | -c <URL> ]`
:   The URL of a binary cache location to which built package(s)
    should be copied.

    If not provided will attempt to read the `cache_url` config value.

`[ --public-cache-url <URL> | -s <URL> ]`
:   The URL of a cache from which built packages will be served at
    installation time.
    This URL typically refers to the same underlying resource
    as specified by the `--cache-url` argument, but using
    a different transport. For example, to upload packages
    to a (writable, authenticated) `s3://` URL,
    but download these packages from an (read-only,
    unauthenticated) `https://cache.floxdev.com endpoint``.

    If not provided the `--public-cache-url` argument will default to
    the `public_cache_url` config value,
    or same value as provided for the `--cache-url` argument.

`[ --signing-key <file> | -k <file> ]`
:   Used for identifying the path to the private key
    to be used to sign packages before upload.
    If not provided the, will default to the `sign_key` config value.


`--prefer-https`
:   Resolve `github:` urls to `https://github.com`,
    instead of `ssh://git@github.com`.

When invoked without arguments, will prompt the user for the required values.

# USAGE OF PUBLISHED PACKAGES

Once published to a channel repository, you can then
search for and use your package with the following:

* subscribe to the channel: `flox subscribe <channel> <URL>`
* search for a package: `flox search -c <channel> <package>`
* install a package: `flox install <channel>.<package>`
