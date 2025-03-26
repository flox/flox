---
title: FLOX-PUBLISH
section: 1
header: "Flox User Manuals"
...

```{.include}
./include/experimental-warning.md
```
> Feature flag: `publish`

# NAME

flox-publish - Publish local packages for Flox


# SYNOPSIS

``` bash
flox [<general-options>] publish
     [-d=<path>]
     [--store-url]
     [--signing-key]
     [<package>]...
```

# DESCRIPTION

Publish the specified `<package>` from the environment in `<path>`,
and output build artifacts.

## Publishing process

Possible values for `<package>` are all keys under the `build` attribute
in the `manifest.toml` and you must specify one.

When publishing a package,
Flox will send the package metadata to the catalog
and optionally upload the package binaries to the store indicated.
This allows re-use of the package in other environments.

Flox makes some assertions before publishing, specifically

- The Flox environment used to build the package is tracked as a git repository.
- Tracked files in the repository are all clean.
- The repository has a remote defined and the current revision has been pushed to it.
- The build environment must have at least one package installed.

Flox will then perform a clone of the repository
to a temporary location
and perform a clean `flox build` operation.
This ensures that all files
required to build the package are included in the git repo.

Upon completion,
the package closure is signed with the key file provided in `--signing-key`
and uploaded to the location specified in `--store_url`.

## After publishing

After publishing,
the package will be availble for `search`, `show`, and `install` operations
like any other package.
The package will be published
to the catalog named as your FloxHub user handle.
To distinguish these packages
from base catalog pacakges,
the name is prefixed with your catalog name.
If your github name was `jsmith` for example,
published packages would be prefixed with `jsmith/`.
If you published a package called `foo`,
you could _search_ and see `jsmith/foo` in the results.
Likewise, you could install the package as
`jsmith/foo`.
The package will be downloaded from the location where it was uploaded.

## Store Location and Authorization

Currently Flox only supports S3 compatibile store locations,
and defers authorization to the nix AWS provider.

Flox uses nix's S3 provider to perform the uploads and downloads,
so you need to be authenticated with AWS
to allow for this.
Using the `awscli2` package (as found in Flox),
you need to run `aws sso login`.
If you are using non-default profiles (see `~/.aws/config`),
you should set AWS_PROFILE in your shell
so `aws` CLI and Flox invocations
use the same AWS profile.

Instructions for setting up the AWS CLI
can be found [here](https://docs.aws.amazon.com/cli/latest/userguide/getting-started-quickstart.html).

## Config options

To simplify the command line during publish,
you can set the `store_url` and `signing_key`
in the Flox config:

``` bash
flox config --set publish.store_url "s3://my-bucket-name"
flox config --set publish.signing_key "/home/<name>/.config/my-flox-catalog.key"
```

## Signing key

If you provide a signing key to Flox,
it will pass this on to Nix
and be used to sign the closure
with that key.
In order to install packages,
nix must be configured to trust this key.

To generate a key pair,
you can use the following commands.
You will use the secret key for publishing
and install the public key on all hosts
where you intend to install this packages
signed by it.

``` bash
# This is the key file you pass to --signing-key
nix key generate-secret --key-name mytest > mytest.key

# Put this public key in `/etc/nix/nix.conf` as an `extra-trusted-public-keys` and restart the nix-daemon
nix key convert-secret-to-public < mytest.key > mytest.pub
```

## Sharing published packages

You are only able to publish packages to your own catalog.
By default only you can see and use these packages.
To allow others
to search and install the packages in you catalog,
you will need to add thier github handles
to a allowlist of users allowed to read from your catalog.

Currently this is managed by a CLI utility shared
[here](https://github.com/flox/catalog-util).
Only the owner of the catalog can manage this list.
See the README of that repository
for additional details.

# OPTIONS

`<package>`
:   The package to publish.
    Possible values are all keys under the `build` attribute
    in the environment's `manifest.toml`.

`--store-url <url>`
:   The store location to upload and download from.
    Currently this must be an S3 bucket like
    `s3://my-bucket`.

`--signing-key <path>`
:   The private key to use in signing the packge
    during upload.  This is a local file path.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# EXAMPLES

`flox publish` is an experimental feature.
To use it the `publish` feature flag has to be enabled:

```shell
$ flox config --set-bool features.publish true
# OR
$ export FLOX_FEATURE_PUBLIsH=true
```

# SEE ALSO

[`flox-activate(1)`](./flox-activate.md)
[`manifest.toml(5)`](./manifest.toml.md)
