---
name: working-with-manifests
description: Use when writing code that reads, writes or migrates a manifest or lockfile, in any crate, or when changing the manifest schema.
---

## Manifest usage (`flox-manifest` crate)

The `flox-manifest` crate uses a type-state pattern (`Manifest<S>`) to enforce
correct manifest lifecycle at compile time. Follow these rules strictly.

- **New schema version for shape changes** - any change to the manifest schema
  (adding, removing, or renaming fields/sections/tables) requires creating a
  new schema version. Never modify an existing schema version's structure.

- **Adding new schemas** - copy the latest `flox-manifest/src/parsed/v*.rs` to
  a new version file and duplicate modified leaf types. Unmodified types
  continue to live in `parsed::common` or their respective version.

- **Always use `Manifest` constructors** - don't pass manifest content as
  `String` or deserialize into inner types directly (e.g.
  `toml_edit::de::from_str::<ManifestLatest>()`). Any manifest read from disk
  or received as text must be migrated. Use the typed constructors:
  - `Manifest::read_typed(path)` / `Manifest::parse_toml_typed(s)` →
    `Manifest<Validated>`
  - `Manifest::read_and_migrate(path, lockfile)` /
    `Manifest::parse_and_migrate(s, lockfile)` → `Manifest<Migrated>`
  - `Manifest::parse_json(s)` for lockfile-embedded manifests →
    `Manifest<TypedOnly>`

- **Never serialize manifests by hand** - don't use
  `toml_edit::ser::to_string()` on inner types. Use
  `manifest.as_writable().to_string()` or
  `manifest.as_writable().write_to_file(path)`, which handle schema version
  selection and format preservation.

- **Outside `flox-manifest`, operate on `ManifestLatest`** - do not introduce
  or expand adapter traits like `CommonFields` so callers can pretend all
  schema versions are interchangeable. The intended model is: migrate to the
  latest schema, then operate on `ManifestLatest`.

- The pattern for `PackageLookup` and `SchemaVersion` doesn't quite match the
  pattern we want to use for operating on `ManifestLatest`, but for now we'll
  keep using the `PackageLookup` trait.

- **Use lockfile migration helpers** - when reading manifest data from a
  `Lockfile`, prefer `lockfile.migrated_manifest()` for the merged manifest and
  `lockfile.migrated_user_manifest()` for the user-authored manifest instead of
  calling `lockfile.manifest.migrate_typed_only(...)` directly at call sites.
