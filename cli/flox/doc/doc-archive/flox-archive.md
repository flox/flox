---
title: FLOX-ARCHIVE
section: 1
header: "Flox User Manuals"
...


# NAME

flox-archive - Archive a package so that it can no longer be installed by name


# SYNOPSIS

```text
flox [<general-options>] archive
     [--reason <text>]
     [--replacement <package>]
     [-f]
     <catalog>/<package>
```

# DESCRIPTION

Archive a package that you no longer maintain, or that another package
replaces.

An archived package is hidden from search and can no longer be installed by
name.
Environments that already have it installed keep working.
You can reverse this at any time with `flox unarchive`.

Name the package with its catalog, such as `myorg/hello`.
Packages in the base catalog cannot be archived.
You need write access to the catalog, and a FloxHub login from
`flox auth login` or a token (see <https://go.flox.dev/auth>).

`flox archive` asks for confirmation before it archives the package,
and refuses to run outside an interactive terminal unless run with
`--force`.

You can record a reason and a replacement with `--reason` and
`--replacement`.
To change them on an archived package, unarchive it, then archive it again.
Archiving it again with the same values does nothing, and archiving it with
different values fails.

# OPTIONS

## Archive Options

`--reason <text>`
:   Why the package is archived.

`--replacement <package>`
:   A package to suggest instead, shown to anyone who tries to install the
    archived package by name.
    Give a package in a user catalog as `<catalog>/<package>`, and a package
    in the base catalog by its name alone (e.g. `hello`).
    The replacement is not checked, so make sure that it exists.

`-f`, `--force`
:   Archive without asking for confirmation.

```{.include}
./include/general-options.md
```

# EXAMPLES

Archive a package that has moved to another catalog:

```bash
flox archive --reason "Moved to myorg" --replacement myorg/hello myuser/hello
```

Archive a package that a base-catalog package replaces:

```bash
flox archive --replacement hello myorg/hello
```

Archive a package without confirmation, such as from a script:

```bash
flox archive -f myorg/old-tool
```

# SEE ALSO

[`flox-unarchive(1)`](./flox-unarchive.md),
[`flox-publish(1)`](./flox-publish.md)
