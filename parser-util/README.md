# `parser-util`

This project provides a minimal executable which exposes various Nix parsers
( and related utilities ) with trivial string and JSON input/output.

## Parsers

### `-r`  `parseAndResolveRef`
Accepts a flake-ref as an attribute set or URI string, performs resolution
on indirect inputs, and prints a JSON object containing the original and
resolved refs in string and _exploded_ attribute set form.

This routine does not attempt to fetch the given URI and simply parses it
and attempts resolution.
This routine is faster than `-l` and will NOT throw an error if its
inputs do not exist.


### `-l`  `lockFlake`
An extended form of `-r` which fetches and _locks_ the given input.

This routine is slower than `-r` and will throw an error if its
inputs do not exist.


### `-i` `parseInstallable`
Parse an _installable_ URI, which is essentially a flake-ref followed by
a `#` character with an attribute path and optionally a `^out1,out2,...`
list of outputs.

The resulting `outputs` field will be either a list of strings
corresponding to the given outputs if they were provided.
If no `^` is given the string "default" is emitted.
If `^*` is given the string "all" is emitted.


### `-u` `parseURI`
Exposes the _low level_ URI parser used by `nix` which explodes a URI
string into its components such as `authority`, `scheme`, `path`, etc.

Notably query strings are exploded into an attribute set.

The fields `authority` and `scheme.application` do not appear in some URIs
and may be either `null` or a `string`.


## Invocation

```
parser-util [-r|-l|-i|-u] <URI|JSON-ATTRS>
parser-util [-h|--help|--usage]
```

The flags listed above may be given to indicate which parser should be used
following by a single argument with a URI string or JSON string.

If no flag is given `-i` will be used if the argument contains a `#`
character, otherwise `-r` is used.
In practice we advise that any scripts which use this utility explicitly
provide the appropriate flag, it is optional for convenient
interactive usage.


## Example Usage

### Resolved Reference

``` shell
$ parser-util -r 'flake:nixpkgs/23.05?dir=lib'|jq;
{
  "input": "flake:nixpkgs/23.05?dir=lib",
  "originalRef": {
    "attrs": {
      "dir": "lib",
      "id": "nixpkgs",
      "ref": "23.05",
      "type": "indirect"
    },
    "string": "flake:nixpkgs/23.05?dir=lib"
  },
  "resolvedRef": {
    "attrs": {
      "dir": "lib",
      "owner": "NixOS",
      "ref": "23.05",
      "repo": "nixpkgs",
      "type": "github"
    },
    "string": "github:NixOS/nixpkgs/23.05?dir=lib"
  }
}

$ parser-util -r '{
    "type": "indirect"
  , "id":   "nixpkgs"
  , "ref":  "23.05"
  , "dir":  "lib"
  }'|jq
{
  "input": {
    "dir": "lib",
    "id": "nixpkgs",
    "ref": "23.05",
    "type": "indirect"
  },
  "originalRef": {
    "attrs": {
      "dir": "lib",
      "id": "nixpkgs",
      "ref": "23.05",
      "type": "indirect"
    },
    "string": "flake:nixpkgs/23.05?dir=lib"
  },
  "resolvedRef": {
    "attrs": {
      "dir": "lib",
      "owner": "NixOS",
      "ref": "23.05",
      "repo": "nixpkgs",
      "type": "github"
    },
    "string": "github:NixOS/nixpkgs/23.05?dir=lib"
  }
}
```


### Locked Flake

``` shell
$ parser-util -l 'flake:nixpkgs/23.05?dir=lib'|jq;
{
  "input": "flake:nixpkgs/23.05?dir=lib",
  "lockedRef": {
    "attrs": {
      "dir": "lib",
      "lastModified": 1685566663,
      "narHash": "sha256-btHN1czJ6rzteeCuE/PNrdssqYD2nIA4w48miQAFloM=",
      "owner": "NixOS",
      "repo": "nixpkgs",
      "rev": "4ecab3273592f27479a583fb6d975d4aba3486fe",
      "type": "github"
    },
    "string": "github:NixOS/nixpkgs/4ecab3273592f27479a583fb6d975d4aba3486fe?dir=lib"
  },
  "originalRef": {
    "attrs": {
      "dir": "lib",
      "id": "nixpkgs",
      "ref": "23.05",
      "type": "indirect"
    },
    "string": "flake:nixpkgs/23.05?dir=lib"
  },
  "resolvedRef": {
    "attrs": {
      "dir": "lib",
      "owner": "NixOS",
      "ref": "23.05",
      "repo": "nixpkgs",
      "type": "github"
    },
    "string": "github:NixOS/nixpkgs/23.05?dir=lib"
  }
}
```


### Plain URIs

``` shell
$ parser-util -u 'flake:nixpkgs/23.05?dir=lib'|jq;
{
  "authority": null,
  "base": "flake:nixpkgs/23.05",
  "fragment": "",
  "path": "nixpkgs/23.05",
  "query": {
    "dir": "lib"
  },
  "scheme": {
    "application": null,
    "full": "flake",
    "transport": "flake"
  }
}
```


### Installables

``` shell
$ parser-util -i 'nixpkgs/23.05#sqlite^bin,dev,out,debug'|jq;
{
  "attrPath": [
    "sqlite"
  ],
  "input": "nixpkgs/23.05#sqlite^bin,dev,out,debug",
  "outputs": [
    "bin",
    "debug",
    "dev",
    "out"
  ],
  "ref": {
    "attrs": {
      "id": "nixpkgs",
      "ref": "23.05",
      "type": "indirect"
    },
    "string": "flake:nixpkgs/23.05"
  }
}

$ parser-util -i 'nixpkgs/23.05#sqlite^*'|jq;
{
  "attrPath": [
    "sqlite"
  ],
  "input": "nixpkgs/23.05#sqlite^*",
  "outputs": "all",
  "ref": {
    "attrs": {
      "id": "nixpkgs",
      "ref": "23.05",
      "type": "indirect"
    },
    "string": "flake:nixpkgs/23.05"
  }
}

$ parser-util -i 'nixpkgs/23.05#sqlite'|jq;
{
  "attrPath": [
    "sqlite"
  ],
  "input": "nixpkgs/23.05#sqlite",
  "outputs": "default",
  "ref": {
    "attrs": {
      "id": "nixpkgs",
      "ref": "23.05",
      "type": "indirect"
    },
    "string": "flake:nixpkgs/23.05"
  }
}
```

## Output Formats

### `-r`  `parseAndResolveRef`

```
{
  "input": <STRING> | <ATTRS>
, "originalRef": {
    "attrs": <ATTRS>
  , "string": <STRING>
  }
, "resolvedRef": ( null | {
    "attrs": <ATTRS>
  , "string": <STRING>
  } )
}
```

`resolvedRef` may be null if you fail to resolve an indirect _flake-ref_.
In such a case we still emit the parsed reference in `originalRef`.


### `-l`  `lockFlake`

```
{
  "input": <STRING> | <ATTRS>
, "originalRef": {
    "attrs": <ATTRS>
  , "string": <STRING>
  }
, "resolvedRef": {
    "attrs": <ATTRS>
  , "string": <STRING>
  }
, "lockedRef": {
    "attrs": <ATTRS>
  , "string": <STRING>
  }
}
```


### `-i`  `parseInstallable`

```
{
  "input": <STRING>
, "attrPath": [<STRING>...]
, "outputs": ( "all" | "default" | [<STRING>...] )
, "ref": {
    "attrs": <ATTRS>
  , "string": <STRING>
  }
}
```


### `-u` `parseURI`

```
{
  "authority": ( null | <STRING> )
, "base": <STRING>
, "fragment": <STRING>
, "path": <STRING>
, "query": { <KEY>: ( <STRING> | null )... }
, "scheme": {
    "application": ( null | <STRING> )
  , "full": <STRING>
  , "transport": <STRING>
  }
}
```
