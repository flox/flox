# Prior-release lockfile baselines

These fixtures are lockfiles produced by an older Flox release.
New-release predicates (`lockfile_if_up_to_date`, `needs_rebuild`)
and `lock()` output must accept them as up-to-date and produce
byte-identical output.

## Current status

The `CAPTURE_PENDING` marker file indicates that captured lockfile
artifacts have not yet been recorded for a real prior Flox release.
The `manifest.toml` files under `plain/` and `with_include/` are
hand-authored templates ready for fixture capture.
Unit tests that depend on these lockfiles are marked `#[ignore]`
and the bats test skips automatically until capture is complete.

After capture:
1. Replace `plain/manifest.lock` with the prior-release output.
2. Add `plain/catalog_replay.yaml` from the capture session.
3. Replace `with_include/parent/manifest.lock` similarly.
4. Add `with_include/parent/catalog_replay.yaml`.
5. Update `MANIFEST.json` with the version metadata.
6. Delete the `CAPTURE_PENDING` marker file.
7. Remove `#[ignore]` from the unit tests.
8. Commit with: `test(ai-159): capture prior-release baselines <version>`

## When to refresh

Refresh when:
- A new minor release of Flox ships (move the pin forward by
  one minor); OR
- The lockfile schema bumps; OR
- A predicate-rejection test fails with a fixture-rot diagnostic.

## How to refresh

```
just regen-prior-release-fixtures [version]
```

This documents the capture procedure.
See the Justfile recipe for the step-by-step process.

After running the recipe:
1. Inspect the diff under `prior_release_baselines/`.
   - `manifest.toml` should be unchanged unless the fixture
     template was edited.
   - `manifest.lock` and `catalog_replay.yaml` will reflect
     the prior-release output.
   - `MANIFEST.json` should record the new version and date.
   - `CAPTURE_PENDING` should be deleted.
2. Run `just unit-tests` in the repo root and confirm the
   AI-159 follow-up unit tests pass.
3. Run the bats test:
   `just integ-tests -- --filter prior-release`
4. Commit with:
   `test(ai-159): capture prior-release baselines to <version>`

## Pin choice

Default pin is the most recent minor release prior to the
current trunk (N-1 minor). Override by passing an explicit
version to the recipe.

## Fixture shapes

| Shape        | Path                         | Covers             |
|--------------|------------------------------|--------------------|
| plain        | `plain/`                     | AI-159-1, -2, -3   |
| with_include | `with_include/`              | AI-159-1, -2       |
| v1 schema    | `migration_baselines/v1/hello/` | AI-159-2        |

The `plain` shape uses `manifest.lock` directly for the
`needs_rebuild` predicate test (AI-159-3). No separate
`rendered_stamp.json` is needed; the test invokes `build()`
which creates the stamp and then checks `needs_rebuild()`.
