
# Lockfiles

## Schema

```
LockedFlakeAttrs ::= {
  lastModified = <INT>
, narHash      = <HASH>
, owner        = <STRING>
, repo         = <STRING>
, rev          = <STRING>
, type         = <STRING>
}

LockedInput ::= {
  attrs       = LockedFlakeAttrs
, fingerprint = <HASH>
, url         = <STRING>
}

LockedPackage ::= {
  attr-path = [<STRING>, ...]
, info      = {<STRING>: <STRING>, ...}
, input     = LockedInput
, priority  = <INT>
}

SystemPackages ::= { <STRING>: null | LockedPackage, ...}

Lockfile ::= {
  manifest         = Manifest
, registry         = Registry
, packages         = { System: SystemPackages, ...}
, lockfile-version = <STRING>
}
```

Fields:
- `LockedFlakeAttrs`
    - These are the typical flake attributes (`owner`, `repo`, etc) with the addition of a `narHash` and a `lastModified` date.
- `LockedInput`
    - `fingerprint`: The fingerprint of a flake, which is a hash suitable for a cache key.
    - `url`: The URL form of the flake reference for this flake.
- `LockedPackage`
    - `attr-path`: The attribute path of this package within the locked input that it comes from.
    - `info`: A collection of metadata for this package.
        - Includes things like `broken`, `license`, `unfree`, `pname`, etc.
    - `priority`: The priority to be used to resolve file conflicts when the environment is built.
