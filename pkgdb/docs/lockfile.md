
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
    - These are the typical flake attributes (`owner`, `repo`, etc) with the
      addition of a `narHash` and a `lastModified` date.
- `LockedInput`
    - `fingerprint`: The fingerprint of a flake, which is a hash suitable for a
       cache key.
    - `url`: The URL form of the flake reference for this flake.
- `LockedPackage`
    - `attr-path`: The attribute path of this package within the locked input
      that it comes from.
    - `info`: A collection of metadata for this package.
        - Includes things like `broken`, `license`, `unfree`, `pname`, etc.
    - `priority`: The priority to be used to resolve file conflicts when the
      environment is built.


## Locking Groups

Currently, we enforce compatibility with package groups in a simple manner;
in the future we expect to have more sophisticated strategies for ensuring
compatibility among group members - but for now we enforce that all group
members come from a single input+rev.


### Strategies

The manifest option `options.package-grouping-strategy` will eventually be used
to select between and customize a variety of grouping strategies; but today
we use the process described above.


#### Current Strategy

The first configurable we'll likely introduce is an option which controls the
treatment of descriptors that do not explicitly set `packageGroup`.
Today we implicitly add all of these descriptors to a single _default_ group.

This means that users must add `packageGroup` to any packages they want to
use a different input+rev than _most other packages_ in their environment.

An alternative strategy would be to implicitly put all of these packages in
their own groups, meaning that users should explicitly indicate which packages
must be compatible with one another.
This has yet to be implemented.


### Extending an Existing Locked Group

When locking new members in a group of packages we aim to preserve existing
resolutions without upgrading other group members; however there are some
interesting situations that are worth explicitly specifying:

1. When we can resolve new descriptors in the group's existing input+rev, we
   do so without modifying any other group members.
2. If we cannot resolve a new descriptor in the group's existing input+rev and
   the descriptor is marked `optional = true`, **we skip it**!
3. If we cannot resolve a new descriptor in the group's existing input+rev, but
   we can resolve all group members in a different input+rev, we modify all
   group members to use the new input+rev.
   a. This emits an info/log message to `STDERR` like "upgrading group `X'..."
      to notify the user that existing packages they're using were changed.
   b. XXX: This auto-upgrade behavior is trivially replaced with a more
           detailed warning message and an explicit operation if desired.
4. If we cannot resolve new descriptors in any inputs we throw an exception
   with a list of descriptors which failed to resolve, and the inputs+revs they
   failed to resolve in.
   a. XXX: The exception message emitted here might need to be abbreviated if
           we start using large numbers of inputs+revs.
