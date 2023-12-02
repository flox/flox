# Manifests

Manifests contain the configuration for an environment, including which packages to install, how to run the shell hook, etc.
Users working inside of a project have both a global manifest as well as a per-project manifest.
Locking these manifests produces a lockfile, which is then used to build the environment.
We also pass these manifests and the lockfile to search commands so that searches can be done in the context of the particular environment e.g. so that we don't return a search result from a registry that isn't available in the project.

## Schema

### Global manifest
```
Allows ::= {
  unfree   = null | <BOOL>
, broken   = null | <BOOL>
, licenses = null | [<STRING>, ...]
}

Semver ::= {
  prefer-pre-releases = <BOOL>
}

Options ::= {
  systems                   = null | [<STRING>, ...]
, allow                     = null | Allows
, semver                    = null | Semver
, package-grouping-strategy = null | <STRING>
, activation-strategy       = null | <STRING>
}

GlobalManifest ::= {
  registry = null | Registry
, options  = null | Options
}
```

Fields:
- `Allows`
  - `unfree`: Allow packages with non-FOSS licenses in search results and installs.
    - Default is `false`.
    - Associated with the `nixpkgs` `meta.unfree` field.
    - Packages that lack this field are assumed to be "free".
  - `broken`: Allow packages marked `broken` in a in search results and installs.
    - Default is `false`.
    - Associated with the `nixpkgs` `meta.broken` field.
    - Packages without this field are assumed not to be broken.
  - `licenses`: A whitelist of software licenses to allow in search results and installs.
    - Default is to allow any license. This default is used if the attribute is missing or `null`.
    - Valid entries are [SPDX Identifiers](https://spdx.org/licenses).
- `SemVer`
  - `prefer-pre-releases`: Whether to prefer pre-release software over equivalent stable versions for the purpose of search results and installs.
    - Default value is `false`, which would prefer `4.1.9` over `4.2.0-pre`.
- `Options`
  - `package-grouping-strategy`: Governs how packages not explicity requested to resolve in a group should be resolved.
    - This field is currently unused.
  - `activation-strategy`: Governs how environments should be activated.
    - This field is currently unused.
- `GlobalManifest`
  - `registry`: Contains the inputs from which packages can be searched and installed from.
    - Users are currently not allowed to put anything in this field, and instead it's inserted when the `--ga-registry` flag is passed to `pkgdb`.
    - For more details on registries see the registry docs.


### Manifest

```
EnvBase ::= {
  floxhub = null | <STRING>
, dir     = null | <STRING>
}

Hook ::= {
  script = null | <STRING>
, file   = null | <PATH>
}

Descriptor ::= {
  name               = null | <STRING>
, optional           = null | <BOOL>
, package-group      = null | <STRING>
, version            = null | <STRING>
, semver             = null | <STRING>
, systems            = null | [<STRING>, ...]
, path               = null | <STRING> | [<STRING>, ...]
, abs-path           = null | <STRING> | [<STRING>, ...]
, package-repository = null | <STRING> | FlakeAttrs
, priority           = null | <INT>
}

Manifest ::= <GlobalManifest> // {
  env-base = null | EnvBase
, install  = null | {<NAME>: Descriptor, ...}
, vars     = null | {<NAME>: <VALUE>, ...}
, hook     = null | Hook
}

```

Fields:
- `EnvBase`
  - `floxhub`: A URL that points to an environment to be extended.
  - `dir`: The path to a directory containing a `.flox` directory to be extended.
- `Hook`
  - `script`: An inline script to run immediately after the environment is activated.
  - `file`: A path to a file containing a script to run immediately after the environment is activated.
- `Descriptor`: A set of requirements specifying a dependency to be installed to the environment.
  - `name`: Matches the `name`, `pname`, or `attrName` attribute of the package.
    - Maps to `flox::pkgdb::PkgQueryArgs::pnameOrAttrName`.
  - `optional`: Whether resolution of this package is allowed to fail without producing an error.
    - The default value is `false`, and is used when the attribute is missing or `null`.
  - `package-group`: Resolve all packages within this group to a single revision of the input they are found in.
  - `version`: Match the version of the package.
    - A `version` whose first character is `=` will attempt to match exactly during resolution.
    - All other `version` strings will attempt to match a semantic version range.
  - `semver`: Specifically match a semantic version range.
    - Uses the exact syntax used by `npm`, `yarn`, etc.
  - `systems`: A list of systems on which to resolve this package.
    - When omitted or `null` only the current system is used for resolution.
  - `path`: Match a relative path within the registry input.
  - `abs-path`: Match an exact path within the registry input.
  - `package-repository`: The named registry input or flake reference that this package should be resolved in.
  - `priority`: A priority used to resolve file conflicts.
    - When the attribute is missing or `null`, the package is assigned a priority of 5.
    - Packages with a higher priority will take precedence over packages with a lower priority.
- `Manifest`
  - `Manifest` is a superset of the fields allowed in `GlobalManifest`.
  - `install`: The collection of `Descriptors` indicating which packages to install.
  - `vars`: The collection of environment variables to set in the environment.
