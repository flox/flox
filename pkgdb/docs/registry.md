# PkgDb Registry

A _registry_ in `pkgdb` is a structure used to organize a set of `nix` _flakes_,
called _inputs_ throughout this document, which have been assigned short-names
and additional metadata.

While the _registry_ structure in `pkgdb` is similar to that used by `nix`, it 
has been extended to record additional information for each _input_ related to
package resolution/search.

Additionally the registry carries a small number of _global_ settings such as
`priority` ( resolution/search ranking for each _input_ ), and
_default_/fallback settings to be used if/when _inputs_ do not explicitly
declare some settings.


## Schemas

Before diving into the details of individual parts of the schema, lets start
with an example of a registry with three _inputs_.
Here we use JSON, but any trivial format could be used.

```json
{
  "inputs": {
    "nixpkgs": {
      "from": {
        "type": "github"
      , "owner": "NixOS"
      , "repo": "nixpkgs"
      , "rev":  "e8039594435c68eb4f780f3e9bf3972a7399c4b1"
      }
    , "subtrees": ["legacyPackages"]
    }
  , "floco": {
      "from": {
        "type": "github"
      , "owner": "aakropotkin"
      , "repo": "floco"
      }
    , "subtrees": ["packages"]
    }
, "defaults": {
    "subtrees": null
  }
, "priority": ["nixpkgs"]
}
```

At the top level the _abstract_ schema for the whole registry is:

```
Subtree :: "packages" | "legacyPackages"

FlakeRef :: ? URL string or Attr Set ?

InputPreferences :: {
  subtrees    = null | [Subtree...]
}

Input :: {
  from = FlakeRef
  subtrees    = null | [Subtree...]
}

Registry :: {
  inputs   = { <INPUT-NAME> = Input, ... }
, defaults = InputPreferences
, priority = null | [<INPUT-NAME>...]
}
```


## Fields

You must provide at least one `input`.
The _keys_ in this attribute set correspond to keys in the `priority` list,
and are often used in the output of `search` and `resolve` invocations.

The `from` fields in each `input` are _flake references_ like those seen in
`flake.nix` files, being either a URL string or an attribute set representation.
These _flake references_ may be locked or unlocked but
**locking is strongly recommended** for most use cases.

The `priority` list should contain ONLY keys from `inputs`, and is used to
indicate a "high" to "low" priority order for performing resolution and search.
Any `inputs` which are missing from `priority` will be ranked lexicographically
after all explicitly prioritized inputs.

The `defaults` field may be used to set fallback settings for `inputs` members.
Explicit definitions in `inputs` override `defaults` settings.
This is discussed further in the section below.


### Fallbacks

The fields `defaults` and `priority` are optional.

The fields `subtrees` are optional ( everywhere ).

An explicit `null` is treated the same as omitting a field.


If no default or explicit settings are given for `subtrees`
there is fallback behavior which attempts to _do the right thing_ without being
overly eager about scraping _everything_ for each input.
Omitting `subtrees` will cause flakes to use `packages`, and finally
`legacyPackages` - only one output will be searched with this behavior.

