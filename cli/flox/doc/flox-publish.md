---
title: FLOX-PUBLISH
section: 1
header: "Flox User Manuals"
...


# NAME

flox-publish - Publish packages for Flox


# SYNOPSIS

```text
flox [<general-options>] publish
     [-d=<path>]
     [-o=<org>]
     [--signing-private-key <path>]
     [--stability <stability>]
     [<package>]...
```

# DESCRIPTION

Publish the specified `<package>` from the environment in `<path>`,
uploading package metadata and copying the packages so that it is available
in the Flox Catalog.

## Preconditions

Flox makes some assertions before publishing, specifically:

- The Flox environment used to build the package is tracked as a git repository.
- Tracked files in the repository are all clean.
- The repository has a remote defined and the current revision has been pushed to it.
- For a manifest build, the environment must have at least one package in the
  `toplevel` package group: the package is published against the nixpkgs
  revision that group is locked to, and an environment with no such package is
  locked to none.
  A Nix expression build has no such requirement, because the revision it is
  published against comes from the catalog server rather than the environment.

These conditions ensure that the package being built can be located, built,
and reproduced in the future.

## Publishing process

Possible values for `<package>` are all keys under the `build` attribute
in `manifest.toml`.
If only one build is defined in `manifest.toml`, specifying the `<package>` is
unnecessary.
If there are multiple builds defined, you may only publish a single package at
a time and must specify the name when calling `flox publish`.

Flox will then perform a clone of the repository to a temporary location
and perform a clean `flox build` operation.
This ensures that all files required to build the package are included in the
git repository.

When publishing a package, metadata is sent to Flox servers so that
information about the package can be made available in `flox install`,
`flox search`, and `flox show`.
The package itself, along with any other packages it depends on, are uploaded
to the Catalog's configured Catalog Store.
By default, Flox provides and configures a Catalog Store, but you may
optionally provide your own Catalog Store.
Contact Flox directly if you're interested in this option.

Finally, the package is uploaded to the default Catalog, which is named after
your user, but you may specify the catalog to publish to via the `--catalog`
option.

## After publishing

After the package is published, it will be available to the `flox install`,
`flox search`, and `flox show` commands.
The package will appear with a name of the form `<catalog>/<name>`
where `<catalog>` is the name of the catalog it was published to, and `<name>`
is the name of the package as it was defined in the `[build]` section of the
manifest.
The `<catalog>` name is either your user name or the name of the organization
that owns the Catalog.

For instance, if a user `myuser` published a package called `hello` to their
personal Catalog, the package would appear in `flox search` as `myuser/hello`.

When installing the package, it is downloaded directly from the Catalog Store
that it was published to.

Published packages on FloxHub are also available as catalog imports for
Nix expression builds.
Other environments reference the published packages as
`catalogs.<catalog>.<package>` in `.nix` expressions under `.flox/pkgs/`.
These references are pinned by a committed `.flox/catalog.lock` when the
project has one (see `flox build update-catalogs`), and are otherwise
resolved fresh during the build.
`flox publish` submits the entries relevant to the published package along
with the build.

## Sharing published packages

A package published to an individual user's Catalog may only be seen and
installed by that user.
In order to share packages with other users you must create an organization.
See [the organizations concept page](https://flox.dev/docs/concepts/organizations/) for more details on
organizations and how to create them.
Note that this is a paid feature available with Flox for Teams.

# OPTIONS

`<package>`
:   The package to publish.
    Possible values are all keys under the `build` attribute
    in the environment's `manifest.toml`.

`-o, --org <org>`
:   Specify the organization to which an artifact should be published to.
    Takes precedence over the default value of the user's GitHub handle.

`--signing-private-key <path>`
:   The private key to use in signing the package
    during upload.  This is a local file path. This option is only necessary
    when using a Catalog Store not provided by Flox.
    Takes precedence over the value of `publish.signing_private_key` from
    'flox config'.

`--stability <stability>`
:   Perform a Nix expression build using a base package set of the given
    stability as tracked by the catalog server.
    A stability (e.g., `"stable"`) identifies a curated nixpkgs revision
    managed by the catalog server.
    When omitted, the latest `"stable"` revision is used.
    The environment's own packages never determine this revision:
    a Nix expression build resolves against the catalog server's current
    view of the stability, not against whatever nixpkgs revision the
    environment happens to be locked to.
    Cannot be used with manifest builds.

```{.include}
./include/dir-environment-options.md
./include/general-options.md
```

# SEE ALSO

[`flox-build(1)`](./flox-build.md)
[`flox-activate(1)`](./flox-activate.md)
[`manifest.toml(5)`](./manifest.toml.md)
