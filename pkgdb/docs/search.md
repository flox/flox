# pkgdb search

The `pkgdb search` command may be used to search for packages which match a
set of `query` filters in a set of [registry](./registry.md) inputs.

This command accepts JSON input and emits JSON output, which are
described below.


## Input

`pkgdb` search accepts either a path to a JSON file or inline JSON with
the following abstract schema:

```
SearchQuery ::= {
  name       = null | <STRING>
  pname      = null | <STRING>
  version    = null | <STRING>
  semver     = null | <STRING>
  match      = null | <STRING>
  match-name = null | <STRING>
}

SearchParams ::= {
  manifest = null | <STRING> | Manifest
, global-manifest = <STRING> | GlobalManifest
, lockfile = null | <STRING> | Lockfile
, query    = SearchQuery
}
```

- `SearchQuery`
  - `match-name`: Partially match a package's `name` or `pname` fields as well as on a calculated column `attrName`.
    - May not be used with `match`.
    - `attrName` is _the last attribute path element_ for `packages` and `legacyPackages` subtrees.
    - Exact matches cause search results to appear before ( higher ) than partial matches.
  - `match`: Partially match a package's `name`, `pname`, or `description` fields as well as on a calculated column `attrName`.
    - May not be used with `query.match-name`.
    - `attrName` is _the last attribute path element_ for `packages` and `legacyPackages` subtrees.
    - Exact matches cause search results to appear before ( higher ) than partial matches.
  - `semver`: a [node-semver](https://github.com/npm/node-semver#ranges) range filter.
    - These use the exact syntax found in tools such as `npm`, `yarn`, `pip`, and many other package managers.
  - `name`: Exactly match the derivation's `name` attribute.
  - `pname`: Exactly match the derivation's `pname` attribute.
    - For derivations which lack a `pname` field it will be parsed from the derivation's `name` attribute using `builtins.parseDrvName`.
  - `version`: Exactly match a derivation's `version` field.
    - For derivations that lack a `version` field it will be parsed from the derivation's `name` attribute using `builtins.parseDrvName`.
- `manifest`: An optional path to a Manifest, or an inline JSON manifest.
- `global-manifest`: A path to a GlobalManifest or an inline JSON GlobalManifest.
  - Note that this parameter is not optional, whereas `manifest` and `lockfile` are.
- `lockfile`: An optional path to an existing Lockfile, or an inline JSON Lockfile.


See the corresponding documentation for [global manifests](./manifests.md#global-manifest), [manifests](./manifests.md#manifest), and [lockfiles](./lockfile.md) for details on those schemas.


### Example Query

Below is an example query that searches four flakes for
_any package matching the string "hello"_ with _major version 2_ , usable
on `x86_64-linux`.

```json
{
  "global-manifest": "/path/to/global-manifest.toml"
, "query": { "match": "hello", "semver": "2" }
}
```

In the example above we'll make a few observations to clarify how defaults are
applied to `inputs` by showing the _explicit_ form of the same params:

```json
{
  "global-manifest": "/path/to/global-manifest.toml"
, "manifest": null,
, "lockfile": null,
, "query": {
    "name": null
  , "pname": null
  , "version": null
  , "semver": "2"
  , "match": "hello"
  , "match-name": null
  }
}
```

- `inputs.<NAME>.subtrees` was applied to all inputs which didn't explicitly
   specify them.
- `inputs.<NAME>.from` _flake references_ were parsed and locked.
- `allow` and `semver` fields were added with their default values.
- `priority` list added _missing_ inputs in lexicographical order.
  + in this case _lexicographical_ is _alphabetical_ because there are no
    numbers or symbols in the names.
- Missing `query.*` fields were filled with `null`.


## Output

Search results are printed as JSON objects with one result per line ordered
such that _high ranking_ results appear **before** _low ranking_ results.

This single result per line format is printed in chunks as each input is
processed, and is suitable for _streaming_ across a pipe.
Tools such as `jq` or `sed` may be used in combination with `pkgdb search` so
that results are displayed to users _as they are processed_.


Each output line has the following format:

```
Result ::= {
  id           = <INT>
, input        = <INPUT-NAME>
, subtree      = "packages" | "legacyPackages"
, absPath      = [<STRING>...]
, relPath   = [<STRING>...]
, pname        = <STRING>
, version      = null | <STRING>
, description  = null | <STRING>
, license      = null | License
, broken       = true | false
, unfree       = true | false
}
```

Note that because the `input` field only prints the short-name of its input,
it is **strongly recommended** that the caller use _locked flake references_.


### Example Output

For the example query parameters given above, we get the following results:

```json
{"absPath":["legacyPackages","x86_64-linux","hello"],"broken":false,"description":"A program that produces a familiar, friendly greeting","id":6095,"input":"nixpkgs","license":"GPL-3.0-or-later","pname":"hello","relPath":["hello"],"subtree":"legacyPackages","system":"x86_64-linux","unfree":false,"version":"2.12.1"}
```
