---
title: FLOX-UNARCHIVE
section: 1
header: "Flox User Manuals"
...


# NAME

flox-unarchive - Unarchive a package so that it can be installed by name again


# SYNOPSIS

```text
flox [<general-options>] unarchive
     <catalog>/<package>
```

# DESCRIPTION

Unarchive a package that was archived with `flox archive`.
The package appears in search again and can be installed by name.

Name the package with its catalog, such as `myorg/hello`.
You need write access to the catalog, and a FloxHub login from
`flox auth login` or a token (see <https://go.flox.dev/auth>).

`flox unarchive` does not ask for confirmation.
Unarchiving a package that is not archived succeeds without changing it.

# OPTIONS

```{.include}
./include/general-options.md
```

# EXAMPLES

Unarchive a package:

```bash
flox unarchive myorg/hello
```

# SEE ALSO

[`flox-archive(1)`](./flox-archive.md),
[`flox-publish(1)`](./flox-publish.md)
