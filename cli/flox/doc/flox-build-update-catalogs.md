---
title: FLOX-BUILD-UPDATE-CATALOGS
section: 1
header: "Flox User Manuals"
...

# NAME

flox-build-update-catalogs - Update the project catalog lockfile

# SYNOPSIS

```text
flox [<general-options>] build update-catalogs
     [-d=<path>]
```

# DESCRIPTION

Scan the project's Nix expression builds (`.nix` files under `.flox/pkgs/`)
for `catalogs.<catalog>.<package>` references and lock them to
`.flox/catalog.lock`.

There is exactly one catalog lock per project.
It pins the source of every catalog reference the project's expressions
make, resolved as a union in a single request, so that every build of a
revision evaluates against one consistent set of inputs.
Commit the file alongside the manifest.

A committed lock is used exactly as it is found: `flox build` and
`flox publish` never rewrite it.
When the project's expressions gain a reference the lock does not cover,
re-run `flox build update-catalogs` and commit the result.
References the scanner cannot fully resolve are pinned conservatively;
a package newly referenced beneath one of them cannot be detected as
missing from the lock and instead fails during evaluation.
Re-running `flox build update-catalogs` resolves this as well.
Without a committed lock, builds resolve the references of the packages
being built fresh on every invocation.

# OPTIONS

```{.include}
./include/dir-environment-options.md
./include/general-options.md
```

# SEE ALSO

[`flox-build(1)`](./flox-build.md)
[`flox-publish(1)`](./flox-publish.md)
