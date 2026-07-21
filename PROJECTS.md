# PROJECTS.md

Tracking doc for the `flox extension` subsystem — porting the `gh extension`
model to flox. Authoritative design document: [`research/gh_extension_flox.md`](research/gh_extension_flox.md).

## Status Legend

- `[x]` Completed
- `[-]` In Progress
- `[ ]` Not Started
- `[~]` Won't fix / Invalid / False positive

## Branch Discipline

P01–P08 were committed exclusively to `smorin/github-extension-prototype`,
branched from `main` at `0badcdf59` (2026-04-15).

**P11 supersedes that branch.** Work now happens on
`smorin/github-extension-prototype-v2`, branched from `origin/main` at
`dd390e66b` (2026-07-21) — 737 commits later — in a worktree at
`../flox-gh-plugin-extension-v2`. The v1 branch is retained as the source
of the port and for its history; it is not the review target.

No merges to `main`, no PR opened until the branch owner decides.

## Design Constraints

The extension subsystem is **purely additive** to the existing flox
architecture. These rules are binding for all of P01–P07:

1. **No refactors of adjacent code.** Touch an existing file only when a
   task explicitly requires it; leave everything else alone.
2. **New variants on the top-level `Commands` enum are permitted and
   preferred over expanding existing group enums.** Adding
   `Commands::Extension(ExtensionCommands)` is additive — it touches
   only the bottom of the enum and the top-level dispatch `match`.
   Adding under `ManageCommands` would mutate a group that belongs to
   unrelated commands. No new *sibling* to `Commands` (a separate
   parser) is introduced. In P01 the variant carries `#[bpaf(hide)]`
   so it stays out of `flox --help` until the feature leaves prototype
   status (see P07-T05).
3. **No new cross-cutting abstractions.** Reuse `GitCommandProvider`,
   `reqwest::Client`, `fslock::LockFile`, `xdg` paths, `httpmock`,
   `Features`, `Flox` context, and the existing telemetry infra as-is.
4. **No modifications to `flox-activations` or `flox-core`.** `Pinned` mode
   shells out to the existing `FLOX_ACTIVATIONS_BIN` the way
   `cli/flox/src/commands/activate.rs` already does.
5. **No new env-var conventions.** Feature flag piggybacks on the existing
   `FLOX_FEATURES_*` pattern. Activation detection uses
   `_FLOX_ACTIVE_ENVIRONMENTS` from `cli/flox-core/src/activate/vars.rs`.
6. **No new dependencies unless unavoidable.** Check `Cargo.lock` first.
   Known likely-new deps for P04: `flate2` + `tar` + `zip` for archive
   extraction, `sha2` for checksum (verify these aren't already transitively
   present).
7. **Feature flag gates the whole subsystem.** Until P07 flips
   `Features::extensions` default to `true`, non-flag flox builds are
   bit-identical to today.

Full rationale and file-by-file impact table:
[`/Users/stevemorin/.claude/plans/create-a-a-more-velvet-dove.md`](/Users/stevemorin/.claude/plans/create-a-a-more-velvet-dove.md).

## Versioning Note

The versions below (v0.1.0 … v1.0.0) are **feature-internal milestone
markers** for the extension subsystem behind `FLOX_FEATURES_EXTENSIONS`.
They are **not** flox git tags — flox ships as a single binary and its
overall version is independent (already past v1.9). These numbers exist to
track milestone completion inside this doc and do not drive any release
process.

## Roadmap at a Glance

| ID  | Title                                                    | Version | Status |
|-----|----------------------------------------------------------|---------|--------|
| P01 | Skeleton and dispatch                                    | v0.1.0  | `[x]`  |
| P02 | Manifest, layout, local install                          | v0.2.0  | `[x]`  |
| P03 | GitHub source: git/script install                        | v0.3.0  | `[x]`  |
| P04 | GitHub source: binary release install                    | v0.4.0  | `[x]`  |
| P05 | `upgrade --all`, table output, lock polish               | v0.5.0  | `[x]`  |
| P06 | Environment integration (the Flox-unique piece)          | v0.6.0  | `[x]`  |
| P07 | Docs, telemetry, GA                                      | v1.0.0  | `[ ]`  |
| P07a | Example repo: flox-hello-script                         | v1.0.0  | `[x]`  |
| P07b | Example repo: flox-hello-binary (macOS only)            | v1.0.0  | `[x]`  |
| P07c | Example repo: flox-hello-local                          | v1.0.0  | `[x]`  |
| P08 | Discovery — search                                       | v1.1.0  | `[x]`  |
| P09 | Authoring and FloxHub source (v2) — needs more detail    | —       | `[ ]`  |
| P10 | Richer UX and ecosystem (v3) — needs more detail         | —       | `[ ]`  |
| P11 | Rebase onto current main; gate behind `features.beta`    | v1.2.0  | `[~]`  |

---

## [x] Project P01: Skeleton and dispatch (v0.1.0)

**Goal**: Land the extension module tree, register the `flox extension`
subcommand as a stub, and wire the two-phase bpaf parse so that
`flox <name>` can dispatch to a `flox-<name>` executable in the managed
directory. Everything behind the `Features::extensions` flag so default
flox behavior is unchanged.

Maps to research doc §Part 4 M0.

**Out of Scope**
- Any real install/list/remove/upgrade logic (all handlers are stubs).
- Manifest parsing or filesystem layout beyond `find`-by-path.
- Environment-activation modes.

### Tests & Tasks

- [x] [P01-T01] Create the extension provider module tree
      `cli/flox-rust-sdk/src/providers/extensions/{mod,manager,extension,manifest,dispatch,layout,source,github}.rs`
      with empty stubs and re-exports from `mod.rs`.
- [x] [P01-T02] Add `pub mod extensions;` to
      `cli/flox-rust-sdk/src/providers/mod.rs` (one line).
- [x] [P01-T03] Add `extensions: bool` (default `false`) to the existing
      `Features` struct in `cli/flox-rust-sdk/src/flox.rs`; wire the
      `FLOX_FEATURES_EXTENSIONS` env-var override into the existing
      features-loading code path.
- [x] [P01-T04] Create `cli/flox/src/commands/extension/` with
      `mod.rs`, `install.rs`, `list.rs`, `remove.rs`, `upgrade.rs`.
      Define `ExtensionCommands` as a `#[derive(Bpaf)]` enum covering the
      four verbs; handlers are `unimplemented!()` stubs.
- [x] [P01-T05] Add `Extension(ExtensionCommands)` variant to the
      top-level `Commands` enum in `cli/flox/src/commands/mod.rs` with
      `#[bpaf(command, hide)]` (the `hide` keeps it out of
      `flox --help` during the prototype window) and wire a
      `Commands::Extension(args) => args.handle(flox).await` arm in
      the top-level dispatch match.
- [x] [P01-T06] In `cli/flox/src/main.rs` (around line 131 where
      `ParseFailure` is matched), add the two-phase-parse fallback: on
      `ParseFailure::Stderr` where the first positional does not start
      with `-` **and** `flox.features.extensions` is true, call
      `dispatch_external(argv)` which looks up `flox-<name>` under
      `$XDG_DATA_HOME/flox/extensions/flox-<name>/flox-<name>` and then
      `$PATH`.
- [x] [P01-T07] Enforce the feature-flag gate in each verb's
      `handle()` (early-return an error when
      `flox.features.extensions == false`). Combined with the
      unconditional `#[bpaf(hide)]` from P01-T05, non-flag builds
      produce bit-identical help output and behavior to today.
- [x] [P01-TS01] Unit: `Features::extensions` default is `false`; override
      via `FLOX_FEATURES_EXTENSIONS=1` sets it to `true`.
- [x] [P01-TS02] Unit: `dispatch_external` returns `Ok` for a present
      `flox-<name>` under the managed dir, falls back to `$PATH` correctly,
      and returns a `NotFound` error when neither contains it.
- [x] [P01-TS03] Integ bats (new file `cli/tests/extension.bats`): with
      `FLOX_FEATURES_EXTENSIONS=1` and a pre-placed executable
      `flox-hello` under the managed dir, `flox hello` prints the expected
      string.
- [x] [P01-TS04] Integ bats: `flox extension --help` lists
      `install`/`list`/`remove`/`upgrade` when the flag is on; the
      `extension` subcommand is absent from `flox --help` regardless
      of flag state (the `#[bpaf(hide)]` attribute keeps it off
      top-level help until P07-T05 removes `hide`).

### Deliverable

```bash
# With feature flag on and a pre-placed script:
$ FLOX_FEATURES_EXTENSIONS=1 flox hello world
Hello, world!

$ FLOX_FEATURES_EXTENSIONS=1 flox extension --help
Usage: flox extension COMMAND
  install  Install an extension
  list     List installed extensions
  remove   Remove an installed extension
  upgrade  Upgrade installed extensions
```

### Automated Verification

- `just build-cli` succeeds.
- `just unit-tests extensions` passes all P01-TS01/P01-TS02 cases.
- `just integ-tests extension.bats` passes P01-TS03/P01-TS04.

### Manual Verification

- With flag off, `flox --help` is byte-identical to current output (diff
  against a pre-branch snapshot).
- Build without the flag env var set; confirm the `extension` subcommand
  does not appear in help.

### Exit Criterion

`flox hello` runs a pre-placed `flox-hello` script; `flox extension --help`
lists the four subcommands. (Research doc §Part 4 M0.)

---

## [x] Project P02: Manifest, layout, local install (v0.2.0)

**Goal**: Introduce the author-facing manifest (`flox-extension.toml`),
the internal `state.toml`, on-disk layout under `$XDG_DATA_HOME/flox/extensions/`,
the `.lock` file for concurrency, and end-to-end `install .` / `list` /
`remove` for local extensions. This is the first user-visible slice.

Maps to research doc §Part 4 M1.

**Out of Scope**
- Any GitHub fetching (next project).
- Binary or archive handling.
- Environment-activation modes.

### Tests & Tasks

- [x] [P02-T01] In `providers/extensions/manifest.rs`, define
      `AuthorManifest`, `ExtensionMeta`, `BinaryMeta`,
      `EnvironmentBehavior`, `InheritMode`, `OnActive` per research doc
      §2.3 with `serde` derives and sensible `Default` impls.
- [x] [P02-T02] In the same file, define `InstalledState` with fields
      `schema`, `name`, `kind`, `source`, `owner`, `repo`, `host`, `tag`,
      `commit`, `pinned`, `asset_sha256`, `installed_at`, `path`.
- [x] [P02-T03] In `providers/extensions/layout.rs`, implement
      `extensions_root(&Flox) -> PathBuf` (= `flox.data_dir.join("extensions")`),
      `install_dir(&Flox, name)`, `state_path(&Flox, name)`,
      `lock_path(&Flox)` (= `extensions_root/.lock`). Use `flox.data_dir`
      — do not re-derive XDG paths.
- [x] [P02-T04] Wrap install/remove in an `fslock::LockFile` on
      `lock_path`, using the guard pattern from
      `cli/flox-rust-sdk/src/providers/upgrade_checks.rs`.
- [x] [P02-T05] Implement `install_local(&Flox, path) -> Result<Extension>`:
      resolve to absolute path, create a staging dir (`<name>.staging-<uuid>`),
      verify `flox-<name>` executable exists at `<path>/flox-<name>`,
      copy `flox-extension.toml` in if present, write `state.toml` with
      `source = "local"` and `commit = git rev-parse HEAD` if `<path>/.git`
      exists, then atomic rename staging → final.
- [x] [P02-T06] Implement `remove(&Flox, name)`: read state, take the
      lock, `fs::remove_dir_all(install_dir)`.
- [x] [P02-T07] Implement `list(&Flox) -> Vec<Extension>`: scan
      `extensions_root` subdirs, parse each `state.toml`, collect results.
      Lock-free.
- [x] [P02-T08] Wire `install.rs` to accept `.` (CWD) or
      `--from-path PATH` and dispatch to `manager::install_local`. Reject
      `owner/repo`-style specs and other strings with a "GitHub sources
      land in P03" error. Wire `list.rs` and `remove.rs` to call
      `manager::list` and `manager::remove`.
- [x] [P02-T09] Implement the atomic-rename staging helper used by install
      (and reused by upgrade later).
- [x] [P02-TS01] Unit: `AuthorManifest` TOML round-trip including all
      defaults for optional blocks (`[environment]` absent,
      `[extension.binary]` absent).
- [x] [P02-TS02] Unit: `InstalledState` TOML round-trip.
- [x] [P02-TS03] Unit: `layout::extensions_root` returns
      `flox.data_dir.join("extensions")`, using
      `flox::test_helpers::flox_instance()` for the fixture (consistent
      with other SDK provider tests).
- [x] [P02-TS04] Unit: `install_local` happy path under a `tempfile` dir
      containing a fake `flox-hello` script.
- [x] [P02-TS05] Unit: `install_local` rejects if no executable
      `flox-<name>` exists at the repo root.
- [x] [P02-TS06] Unit: concurrent `install` + `remove` using the `fslock`
      guard — the second caller sees `WouldBlock` (or serializes),
      no corruption.
- [x] [P02-TS07] Integ bats: init a fake extension directory,
      `flox extension install .`, `flox extension list` shows it,
      `flox extension remove <name>`, list is empty again.

### Deliverable

```bash
$ mkdir -p /tmp/flox-hello && cat > /tmp/flox-hello/flox-hello <<'EOF'
#!/usr/bin/env bash
echo "hello from $FLOX_EXTENSION_NAME"
EOF
$ chmod +x /tmp/flox-hello/flox-hello
$ FLOX_FEATURES_EXTENSIONS=1 flox extension install --from-path /tmp/flox-hello
Installed flox-hello (local) -> ~/.local/share/flox/extensions/flox-hello

$ FLOX_FEATURES_EXTENSIONS=1 flox extension list
NAME   REPO  VERSION  PINNED
hello  .     -

$ FLOX_FEATURES_EXTENSIONS=1 flox hello
hello from hello

$ FLOX_FEATURES_EXTENSIONS=1 flox extension remove hello
Removed flox-hello
```

### Automated Verification

- `just unit-tests` passes P02-TS01 through P02-TS06.
- `just integ-tests extension.bats` passes P02-TS07.

### Manual Verification

- Install the same extension twice without `--force` and confirm a clean
  "already installed" error.
- Inspect `$XDG_DATA_HOME/flox/extensions/flox-<name>/state.toml` and
  confirm all fields are populated and parseable.

### Exit Criterion

Local developer loop works: authors can iterate on an extension locally
without ever touching GitHub. (Research doc §Part 4 M1.)

---

## [x] Project P03: GitHub source — git/script install (v0.3.0)

**Goal**: First remote-source path. Clone a GitHub repo into the managed
directory as a script/git extension, with `--pin`, `--force`, and
`upgrade <name>` support for the clone-based kinds.

Maps to research doc §Part 4 M2.

**Out of Scope**
- Binary release assets / archive extraction (P04).
- `upgrade --all` (P05).
- FloxHub or non-GitHub sources (v2).

### Tests & Tasks

- [x] [P03-T01] In `providers/extensions/github.rs`, define the
      `GitHubSource` struct holding a `reqwest::Client` (with configurable
      `base_url`) and a `GitCommandProvider`. The `ExtensionSource` trait
      is deferred to P04 where a second implementation may justify the
      abstraction (Design Constraint #3 + CLAUDE.md provider-trait
      guidance).
- [x] [P03-T02] In `providers/extensions/github.rs`, implement
      `GitHubSource` holding a `reqwest::Client` and a
      `GitCommandProvider` (from `cli/flox-rust-sdk/src/providers/git.rs`).
      Default host `github.com`; accept GHE host overrides.
- [x] [P03-T03] `GitHubSource::resolve_latest`: `GET
      https://api.github.com/repos/:owner/:repo/releases/latest`; fall back
      to `GET /repos/:owner/:repo` → `default_branch` → `GET
      /repos/:owner/:repo/commits/<branch>` for the HEAD commit.
- [x] [P03-T04] `GitHubSource::resolve_pin`: if input matches semver/`v*`,
      `GET /releases/tags/<pin>`; else treat as commit-SHA prefix and
      `GET /commits/<prefix>` to expand to a full SHA. Hex pins also
      fetch `default_branch` so `install_github` has a clonable named
      ref (`git clone --branch` rejects raw SHAs); the post-clone
      `git reset --hard <commit>` then pins the worktree exactly.
- [x] [P03-T05] `GitHubSource::clone_repo`: delegate to
      `GitCommandProvider::clone_branch_with(url, staging, branch,
      bare=false)` (single-ref clone, no tags). `--depth=1` is not
      exposed by the existing API and Design Constraint #3 forbids
      extending it for the prototype.
- [x] [P03-T06] Extend `install.rs` with the non-local path: parse
      `owner/repo` or full URL, normalize (ensure repo suffix is
      `flox-<name>`, extract `name`), run `checkValidExtension` (core-name
      collision check + already-installed check), clone into staging,
      verify `flox-<name>` executable, copy `flox-extension.toml`, write
      `state.toml` with `kind = Script` or `Git`, atomic rename.
- [x] [P03-T07] Add `--pin <ref>` flag to `install`; sets `pinned = true`
      in state.
- [x] [P03-T08] Add `--force` flag with dual install/upgrade meaning:
      on `install`, deletes the existing install dir on owner-conflict;
      on `upgrade`, also overrides the `pinned = true` skip (D4).
- [x] [P03-T09] Implement `upgrade.rs` for script/git kinds:
      `GitCommandProvider::fetch_ref(install_dir, "origin", &branch)`
      (no colon — fetches into FETCH_HEAD without updating the local
      branch ref, which git would refuse for the currently checked-out
      branch) then shell out to `git -C <install_dir> reset --hard FETCH_HEAD`
      (matches the P02-D2 precedent of one-off `git` invocations from
      manager.rs). Update `commit` in `state.toml`. Skip with
      `UpgradeStatus::Pinned` if `pinned = true` unless `--force`.
      Local-kind extensions return `UpgradeError::LocalNotSupported`.
- [x] [P03-T10] In `providers/extensions/manager.rs`, add a
      `const RESERVED_COMMAND_NAMES: &[&str]` list of flox's top-level
      subcommand names (e.g., `install`, `activate`, `search`, `init`, …).
- [x] [P03-TS01] Unit: `resolve_latest` against a canned
      `releases/latest` JSON fixture via `httpmock`.
- [x] [P03-TS02] Unit: `resolve_pin` for a tag name and for a commit-SHA
      prefix, both against `httpmock` fixtures.
- [x] [P03-TS03] Unit: `checkValidExtension` rejects a name colliding with
      `RESERVED_COMMAND_NAMES`.
- [x] [P03-TS04] Unit: drift test — walk bpaf help output of the top-level
      `Commands` enum and assert every top-level subcommand is in
      `RESERVED_COMMAND_NAMES`; fail the build if a new command is added
      without updating the list.
- [x] [P03-TS05] Integ bats: init a local bare git repo with a single
      commit containing an executable `flox-hello`, install from that
      repo path (as the remote), verify `flox hello` runs.
- [x] [P03-TS06] Integ bats: `flox extension install --pin <tag>` pins;
      `flox extension upgrade <name>` is a no-op without `--force` and
      re-fetches with `--force`.
- [x] [P03-TS07] Integ bats: `--force` install overwrites a prior install
      from a different owner.

### Deliverable

```bash
$ FLOX_FEATURES_EXTENSIONS=1 flox extension install flox-examples/flox-hello-script
Cloned flox-examples/flox-hello-script at abc1234
Installed flox-hello (script)

$ FLOX_FEATURES_EXTENSIONS=1 flox extension list
NAME   REPO                             VERSION   PINNED
hello  flox-examples/flox-hello-script  abc12345

$ FLOX_FEATURES_EXTENSIONS=1 flox extension upgrade hello
Upgraded flox-hello: abc12345 -> def67890
```

### Automated Verification

- `just unit-tests` passes P03-TS01 through P03-TS04.
- `just integ-tests extension.bats` passes P03-TS05 through P03-TS07.

### Manual Verification

- Install a real public `flox-extension`-topic script repo end-to-end.
- Attempt to install `flox-examples/flox-activate` (if such a name existed)
  and confirm the reserved-name rejection kicks in.

### Exit Criterion

`flox extension install <owner>/flox-<name>` works end-to-end for a
script/git-kind repository. (Research doc §Part 4 M2.)

---

## [x] Project P04: GitHub source — binary release install (v0.4.0)

**Goal**: Precompiled-binary path. Resolve and download a release asset
matching the host's OS/arch, extract archives as needed, record
checksums, and support upgrade.

Maps to research doc §Part 4 M3.

**Out of Scope**
- Checksum *verification* (recorded, not enforced in v1; deferred to v3).
- Build-provenance / attestation.
- Non-GitHub sources.

### Tests & Tasks

- [x] [P04-T01] Implement `GitHubSource::list_release_assets`:
      `GET /releases/tags/<tag>` and parse the `assets[]` array (name,
      `browser_download_url`, size, content type).
- [x] [P04-T02] Implement platform-string computation:
      `format!("{os}-{arch}")` with `os ∈ {linux, darwin, windows}`,
      `arch ∈ {x86_64|amd64, aarch64|arm64}`. Emit both nomenclatures for
      substring matching (so `amd64` and `x86_64` both match).
- [x] [P04-T03] Implement asset resolution in the priority order:
      (1) author-provided `binary.platforms[<platform>]` map,
      (2) rendered `asset_template` from the manifest,
      (3) substring match on asset name,
      (4) `darwin-arm64` → `darwin-amd64` Rosetta fallback (mirrors
      `cli/cli#9592`).
- [x] [P04-T04] Implement `GitHubSource::download_asset`: stream via
      `reqwest` into staging, compute SHA-256 incrementally, record the
      hex digest to `state.toml` via `asset_sha256` inside the install dir.
- [x] [P04-T05] Implement archive extraction for `.tar.gz` (via
      `flate2` + `tar`) and `.zip` (via `zip`); locate the `flox-<name>`
      executable inside the extracted tree; `chmod +x`.
- [x] [P04-T06] Raw-single-file path: if the asset has no archive suffix,
      copy it directly as `flox-<name>` and `chmod +x`.
- [x] [P04-T07] Install flow for `kind = Binary`: resolve tag, resolve
      asset, download to staging, extract/copy, write `state.toml` with
      `tag` and `asset_sha256`, atomic rename.
- [x] [P04-T08] Upgrade flow for binary: re-run `resolve_latest`; if
      `tag == state.tag`, return `UpgradeStatus::AlreadyCurrent`; else
      re-run the install flow in staging and atomic rename.
- [x] [P04-T09] Add `InstallError::NoMatchingAsset { owner, repo,
      platform }` with a hint message directing the user to file an issue
      against the extension repo. (Per-operation enum rather than an
      aggregate `ExtensionError`; same pattern as existing install/upgrade
      errors.)
- [x] [P04-TS01] Unit: asset-resolver truth table — explicit map beats
      template beats substring beats Rosetta fallback.
- [x] [P04-TS02] Unit: platform string generation across
      Linux/macOS/Windows × amd64/arm64, both nomenclatures accepted.
- [x] [P04-TS03] Unit: `.tar.gz` extraction fixture, `.zip` extraction
      fixture, raw single-file fixture.
- [x] [P04-TS04] Unit: recorded SHA-256 matches a pre-computed fixture
      checksum.
- [x] [P04-TS05] Unit: upgrade no-op when `tag == state.tag`.
- [x] [P04-TS06] Integ bats: serve a fake `releases/latest` JSON and a
      tarball asset via a local `http.server`; install end-to-end;
      `flox <name>` runs the extracted binary; upgrade picks up a bumped
      tag.

### Deliverable

```bash
$ FLOX_FEATURES_EXTENSIONS=1 flox extension install flox-examples/flox-tool
Resolved flox-tool v1.0.0
Downloading flox-tool-linux-x86_64.tar.gz (2.1 MB)
SHA256: fe14…cafe
Installed flox-tool v1.0.0 (binary)

$ FLOX_FEATURES_EXTENSIONS=1 flox tool --help
flox-tool 1.0.0 - a flox extension

$ # after upstream cuts v1.0.1:
$ FLOX_FEATURES_EXTENSIONS=1 flox extension upgrade tool
Upgraded flox-tool: v1.0.0 -> v1.0.1
```

### Automated Verification

- `just unit-tests` passes P04-TS01 through P04-TS05.
- `just integ-tests extension.bats` passes P04-TS06.

### Manual Verification

- Install a real published `flox-extension`-topic repo that ships Linux +
  macOS release assets; run its `--help`.
- On an Apple Silicon host, install an extension that ships only
  `darwin-amd64` assets and confirm the Rosetta fallback is used with a
  clear log message.

### Exit Criterion

Install a real published `flox-extension` that ships Linux + macOS
binaries; upgrade picks up a new tag. (Research doc §Part 4 M3.)

---

## [x] Project P05: `upgrade --all`, table output, lock polish (v0.5.0)

**Goal**: Cross-extension operations and the final UX polish before
environment integration. Unified table output for `list` and `upgrade`,
centralized error strings, and a full audit of `.lock` discipline.

Maps to research doc §Part 4 M4.

**Out of Scope**
- Environment-activation modes (P06).
- Telemetry (P07).

### Tests & Tasks

- [x] [P05-T01] Implement `upgrade --all`: iterate every installed
      extension, invoke per-kind upgrade, collect `UpgradeResult { name,
      from, to, status }`.
- [x] [P05-T02] Implement `--dry-run`: run only the resolve step for each
      target and print the would-be plan; no writes to disk.
- [x] [P05-T03] Unified table formatter with columns NAME / REPO /
      VERSION (tag if present, else 8-char truncated commit) / PINNED /
      STATUS. Used by both `list` and `upgrade` (including `--dry-run`).
- [x] [P05-T04] Centralize the user-facing error strings from research
      doc §2.9 as `Display` impls on `ExtensionError` variants:
      - `CommandConflict` → `"error: name '<x>' conflicts with a built-in
        flox command"`
      - `AlreadyInstalled` → `"error: flox-<name> is already installed
        (run with --force to overwrite)"`
      - `NoMatchingAsset` → `"error: no release asset matches '<os-arch>'
        for <owner>/<repo>"` + maintainer-issue hint
      - `ExecutableMissing` → `"error: extension '<name>' has no
        executable at <path>"`
      - `PinnedEnvUntrusted` → `"error: extension '<name>' requires the
        '<env>' environment; trust it with 'flox activate -r <env>
        --trust' first"`
      - Pinned-upgrade skip log line: `"skipping '<name>' (pinned to
        <tag>); pass --force to override"`
- [x] [P05-T05] Audit and tighten `.lock` usage: every install / upgrade
      / remove acquires the exclusive lock; `list` and the `find`-for-
      dispatch path are lock-free; every `state.toml` write is
      temp-file + rename.
- [x] [P05-T06] Add progress indication for binary downloads reusing
      whatever progress infrastructure `publish.rs` / `catalog.rs`
      already use. Do not add a new progress-bar crate.
- [x] [P05-TS01] Unit: `UpgradeStatus` enum mapping covers `Upgraded`,
      `AlreadyCurrent`, `Pinned`, `Failed(reason)`.
- [x] [P05-TS02] Unit: `--dry-run` across all three kinds performs zero
      filesystem writes (assert via a read-only temp dir).
- [x] [P05-TS03] Unit: concurrent `upgrade --all` + `remove` serialize
      cleanly under the lock; no state-file corruption.
- [x] [P05-TS04] Integ bats: pre-seed three extensions (local, git,
      binary), run `flox extension upgrade --all --dry-run`, assert the
      expected table output.
- [x] [P05-TS05] Integ bats: trigger every research-doc §2.9 error
      condition and assert the exact user-facing string.

### Deliverable

```bash
$ FLOX_FEATURES_EXTENSIONS=1 flox extension list
NAME    REPO                           VERSION        PINNED  STATUS
deploy  flox-examples/flox-deploy      v0.3.1         no      update
report  acme/flox-report               v1.2.3         yes
tool    flox-examples/flox-tool        def67890       no      up-to-date

$ FLOX_FEATURES_EXTENSIONS=1 flox extension upgrade --all --dry-run
Plan:
  deploy:   v0.3.1 -> v0.4.0 (would upgrade)
  report:   pinned to v1.2.3 (would skip; pass --force to override)
  tool:     def67890 (already current)
```

### Automated Verification

- `just unit-tests` passes P05-TS01 through P05-TS03.
- `just integ-tests extension.bats` passes P05-TS04 and P05-TS05.

### Manual Verification

- Run `upgrade --all` against a real mixed-kind install set and confirm
  the output matches the designed table format.
- Run two `flox extension install` commands in parallel shells and
  confirm the second blocks/errors cleanly rather than racing.

### Exit Criterion

`flox extension upgrade --all --dry-run` produces the expected report
in a repo with multiple kinds; all §2.9 error strings verified.
(Research doc §Part 4 M4.)

---

## [x] Project P06: Environment integration (v0.6.0) — the Flox-unique piece

**Goal**: Wire the three activation modes (`Inherit`, `None`,
`Pinned(ref)`) into the extension dispatch path. This is the single
substantive divergence of the Flox extension design from `gh extension`.

Maps to research doc §Part 4 M5.

**Out of Scope**
- `on_active = "layer"` (deferred; research doc §1.11 explicitly defers
  this pending clarity on composition semantics).
- Modifying anything inside `cli/flox-activations/` — `Pinned` mode
  spawns the existing binary, same way `cli/flox/src/commands/activate.rs`
  already does.

### Tests & Tasks

- [x] [P06-T01] Implement `dispatch::resolve_mode(manifest_env:
      Option<&EnvironmentBehavior>) -> ActivationMode` per research doc
      §1.11 rules. `ActivationMode` = `Pinned(String) | Inherit | None`.
      The `_FLOX_ACTIVE_ENVIRONMENTS` idempotency check is kept out of
      the SDK (handled at the CLI layer in `try_dispatch_external`) to
      avoid duplicating the `ActiveEnvironments` JSON parser.
- [x] [P06-T02] Detect "inside a flox activation" using
      `_FLOX_ACTIVE_ENVIRONMENTS` (constant
      `FLOX_ACTIVE_ENVIRONMENTS_VAR` from
      `cli/flox-core/src/activate/vars.rs`), not `FLOX_ENV`.
- [x] [P06-T03] `Inherit` mode: spawn the extension child with
      `std::process::Command::new(&ext.executable).args(argv).envs(std::env::vars_os())`;
      parent env already carries all `FLOX_*` vars the environment set
      up.
- [x] [P06-T04] `None` mode: construct the child env by filtering out
      every key matching `^FLOX_` or `^_FLOX_` before spawn.
- [x] [P06-T05] `Pinned(ref)` mode: spawn `FLOX_ACTIVATIONS_BIN` with
      the equivalent of `flox activate -r <ref> -- <ext.executable>
      <argv…>`. Use the in-place replacement mechanism from
      `cli/flox/src/commands/activate.rs` (the same call on line ~502) so
      PID semantics match a normal `flox activate -- <cmd>` invocation.
- [x] [P06-T06] In every mode, inject the bookkeeping env vars:
      `FLOX_EXTENSION_NAME`, `FLOX_EXTENSION_VERSION`,
      `FLOX_EXTENSION_PATH`, `FLOX_BIN` (= `std::env::current_exe()`).
- [x] [P06-T07] When `on_active = "error"` and the user is inside a
      different environment than the one pinned, emit
      `ExtensionError::PinnedEnvMismatch` with a hint pointing at the
      offending extension name. `on_active = "override"` (default) just
      activates.
- [x] [P06-T08] Replace the P01 `dispatch_external` stub in `main.rs`
      with a call to `dispatch::spawn_extension(&ext, argv, &flox)`.
- [x] [P06-T09] Idempotency: if mode is `Pinned(ref)` and the user is
      already inside `<ref>` (parse `_FLOX_ACTIVE_ENVIRONMENTS` and look
      for the ref), skip re-activation and launch the extension directly
      in-place.
- [x] [P06-TS01] Unit: `resolve_mode` truth table — all combinations of
      `InheritMode × {_FLOX_ACTIVE_ENVIRONMENTS unset, present with
      matching ref, present with non-matching ref}` map to the expected
      `ActivationMode`.
- [x] [P06-TS02] Unit: `None`-mode env-scrubbing removes every `FLOX_*`
      and `_FLOX_*` key while preserving others.
- [x] [P06-TS03] Unit: bookkeeping env vars are set on the child
      `Command` object for every mode.
- [x] [P06-TS04] Integ bats: outside any activation, a fixture
      `flox-echo-env` extension prints `FLOX_ENV is unset`; assert.
      (Covers research-doc §1.5 user story #4 negative path.)
- [x] [P06-TS05] Integ bats: inside `flox activate`, the same fixture
      sees the environment's `$FLOX_ENV` and a known env-supplied tool
      on `PATH`. (Covers §1.5 user story #4 positive path.)
- [x] [P06-TS06] Integ bats: pinned-env extension activates the
      manifest's env even when the user is outside any activation.
      (Covers §1.5 user story #5.)
- [x] [P06-TS07] Integ bats: pinned-env extension when user is already
      inside the same env is a no-op — verify via timing or via probe
      output that no nested activation happens.
- [x] [P06-TS08] Integ bats: pinned + `on_active = "error"` + user
      inside a different env → error message contains the offending
      extension name + hint.
- [x] [P06-TS09] Integ bats: `None` mode — extension sees no `FLOX_*`
      vars even when the parent has them. (Covers §1.5 user story #6.)

### Deliverable

```bash
# User story #4 (Inherit, inside activation):
$ flox activate -d ./myproj
flox [myproj] $ FLOX_FEATURES_EXTENSIONS=1 flox greet
Hello from myproj — my PATH includes /nix/store/…-myproj/bin

# User story #5 (Pinned, outside activation):
$ FLOX_FEATURES_EXTENSIONS=1 flox deploy staging
→ activating acme/prod-tools (pinned by flox-deploy manifest)
✓ deploy complete

# User story #6 (None):
$ FLOX_FEATURES_EXTENSIONS=1 flox isolated-extension
(no FLOX_* vars visible in the child's env)
```

### Automated Verification

- `just unit-tests` passes P06-TS01 through P06-TS03.
- `just integ-tests extension.bats` passes P06-TS04 through P06-TS09.

### Manual Verification

- Run `flox deploy` (a fixture extension with a pinned env) from three
  starting conditions: no activation, same-env activation, different-env
  activation. Confirm all three behave per §1.11.

### Exit Criterion

Research-doc §1.5 user stories #4, #5, and #6 pass end-to-end.
(Research doc §Part 4 M5.)

---

## [x] Project P07: Docs, telemetry, GA (v1.0.0)

**Goal**: Ship v1 to users. User + author docs, example repos, telemetry
wired into the existing metrics pipeline, and flip the feature flag to
on-by-default.

Maps to research doc §Part 4 M6.

**Out of Scope**
- `search`, `create`, `browse`, `exec` (v2/v3).
- FloxHub integration (v2).
- Example extension repos (moved to P07a/P07b/P07c as separate
  projects).
- A `flox/flox-extension-precompile` GitHub Action (parallel workstream;
  not blocking GA).

### Tests & Tasks

- [x] [P07-T01] Write the user guide at `docs/extensions/user-guide.md`
      covering install/list/remove/upgrade, pinning, and environment
      inheritance with the three modes.
- [x] [P07-T02] Write the author guide at
      `docs/extensions/author-guide.md` covering repo naming
      (`flox-<name>`), the `flox-extension.toml` schema, the three kinds,
      release-asset naming conventions, and the `[environment]` stanza.
- [~] [P07-T04] Deferred per design review; revisit post-GA. Was:
      emit telemetry events for `install` / `upgrade` / `remove` /
      `dispatch`.
- [x] [P07-T05] Remove the `FLOX_FEATURES_EXTENSIONS` gate by flipping
      the `Features::extensions` default to `true`; keep the env-var
      override for opt-out. In the same commit, **remove the
      `#[bpaf(hide)]` attribute** from the `Commands::Extension`
      variant in `cli/flox/src/commands/mod.rs` so that
      `flox --help` now lists `extension` alongside the other
      subcommands. Delete the handler-side `flox.features.extensions`
      early-return checks in `cli/flox/src/commands/extension/*.rs`
      (the flag is the default; no runtime gate needed anymore).
      Update `cli/tests/extension.bats` P01-TS04 to assert `extension`
      *is* present in `flox --help` after this change.
- [x] [P07-TS01] Docs-presence check — a bats block that asserts
      `docs/extensions/README.md`, `user-guide.md`, and
      `author-guide.md` exist and that every relative markdown link in
      them resolves. Replaces the original docs-build check; no docs
      build exists in-repo.
- [x] [P07-TS02] Integ bats smoke test — a parity check translating a
      subset of `gh` integration-test assertions (`install`, `list`,
      `remove`, `upgrade`) to the `flox extension` surface.
- [~] [P07-TS03] Deferred per design review; revisit post-GA. Was:
      telemetry events match the documented schema.

### Deliverable

- Docs pages exist under `docs/extensions/` (`user-guide.md`,
  `author-guide.md`, `README.md`).
- `flox extension <verb>` works without `FLOX_FEATURES_EXTENSIONS` set.

### Automated Verification

- `just build-cli` succeeds.
- `just unit-tests` passes, including the flipped
  `Features::extensions` default assertion.
- `just integ-tests extension.bats` passes the flipped help test,
  the gh-parity smoke block, and the docs-presence block.
- `grep -r 'flox\.features\.extensions' cli/flox/src/` returns zero
  hits (all five handler gates removed).
- `grep -rn 'FLOX_FEATURES_EXTENSIONS' cli/` shows only the
  deserialization path and the user-guide opt-out reference, not a
  handler gate.

### Manual Verification

- Fresh install the built flox binary; without any env flags, run
  `flox extension install flox-examples/flox-hello-script` and confirm
  end-to-end behavior.
- Run `FLOX_FEATURES_EXTENSIONS=0 flox extension list` and confirm
  the subsystem can still be disabled via env override.

### Exit Criterion

v1 GA. Smoke tests translated from `gh extension` all pass.
(Research doc §Part 4 M6.)

---

## [x] Project P07a: flox-hello-script example repo (v1.0.0)

**Goal**: Publish `flox/flox-hello-script`, the canonical
script-kind reference extension. Linked from the author guide
(P07-T02) and used for manual GA verification of the P03 install
path.

**Out of Scope**
- Precompiled binary assets (see P07b).
- Local-only example (see P07c).
- Use as a fixture for `cli/tests/extension.bats` — those tests use
  local fixtures; this repo is for users.
- Telemetry in the example repo (tracks P07-T04, deferred).
- A flox-binary smoke step in the example repo's CI (avoids a
  bootstrap cycle against GA artifacts; covered by P07a-TS03 here).

### Tests & Tasks

- [x] [P07a-T01] Create the local working tree at
      `~/c/flox_repos/flox-hello-script` (`mkdir -p` the parent
      first) and `git init -b main` inside it.
- [x] [P07a-T02] Add `LICENSE` (Apache-2.0, matching flox) and an
      initial empty-tree-free commit on `main`.
- [x] [P07a-T03] Add `flox-extension.toml` with an `[extension]`
      table: `name = "hello-script"` (matches the derived
      extension name from the `flox-hello-script` repo), a
      one-line `description`, no `[extension.binary]` sub-table
      (install-time kind derivation in `manager.rs:590` lands on
      `script` when `binary` is absent — there is no manifest
      `kind` field), and **no** `[environment]` block — so the
      default `Inherit` mode applies, exercising the P06-T01
      resolver default end-to-end.
- [x] [P07a-T04] Add executable `flox-hello-script` (bash,
      `#!/usr/bin/env bash`, `set -euo pipefail`) that prints
      `Hello from $FLOX_EXTENSION_NAME v$FLOX_EXTENSION_VERSION`
      and echoes any positional args. `chmod +x`. Exercises the
      P06-T06 bookkeeping env vars.
- [x] [P07a-T05] Add `README.md`: one-line description, install /
      upgrade / remove command examples, a link back to the flox
      author guide (`docs/extensions/author-guide.md`), and a note
      that this repo is the canonical P03 script-kind reference.
- [x] [P07a-T06] Add `.github/workflows/ci.yml`: `shellcheck
      flox-hello-script` + a TOML-parse sanity check on
      `flox-extension.toml` (tiny Python `tomllib` script is
      fine). No flox-binary step.
- [x] [P07a-T07] Create the **public** GitHub repo and push via
      `gh repo create flox/flox-hello-script --public
      --source=. --remote=origin --push` from inside
      `~/c/flox_repos/flox-hello-script`. Verify public visibility
      with `gh repo view flox/flox-hello-script --json
      visibility` → `"PUBLIC"`.
- [x] [P07a-T08] Tag `v0.1.0` on the initial commit and push the
      tag (`git tag v0.1.0 && git push origin v0.1.0`).
- [x] [P07a-T09] Apply the `flox-extension` GitHub topic via
      `gh repo edit flox/flox-hello-script --add-topic
      flox-extension` so P08 `search` surfaces it. Verify with
      `gh repo view flox/flox-hello-script --json
      repositoryTopics`.
- [x] [P07a-T10] In **this** repo, edit
      `docs/extensions/author-guide.md` to cross-link the
      published example repo from the script-kind section.
- [x] [P07a-TS01] Example repo CI: `shellcheck flox-hello-script`
      passes.
- [x] [P07a-TS02] Example repo CI: `flox-extension.toml` parses
      as valid TOML and deserializes against the documented
      `AuthorManifest` shape (Python `tomllib` + assertions on
      the `[extension]` keys).
- [x] [P07a-TS03] Integ bats (this repo,
      `cli/tests/extension.bats`): new test case
      `extension: flox/flox-hello-script mirror installs and
      dispatches` that mirrors the example repo's layout into a
      local bare repo via `_setup_hello_script_fixture` and runs
      `flox extension install flox/flox-hello-script`
      end-to-end, then `flox hello-script world` and asserts the
      expected greeting. Guards against schema drift breaking the
      canonical example.

### Deliverable

`flox extension install flox/flox-hello-script` resolves,
clones, and runs end-to-end against a GA flox binary.

### Automated Verification

- Example repo CI is green on `main` (P07a-TS01, P07a-TS02).
- `just integ-tests extension.bats -- --filter hello-script`
  passes the fixture-mirror test (P07a-TS03).

### Manual Verification

- Fresh checkout of flox with the feature flag on; run
  `flox extension install flox/flox-hello-script` and confirm
  `flox hello-script` executes and prints the bookkeeping-env-var
  greeting.
- `flox extension upgrade hello-script` is a no-op immediately
  after install, and picks up a bumped `v0.1.1` tag after one is
  cut.

### Exit Criterion

Repo is public at `github.com/flox/flox-hello-script`, tagged
`v0.1.0`, carries the `flox-extension` topic, is linked from the
author guide, and installs cleanly via `flox extension install
flox/flox-hello-script`.

---

## [x] Project P07b: flox-hello-binary example repo — macOS (v1.0.0)

**Goal**: Publish `flox/flox-hello-binary`, the canonical
precompiled-binary reference extension. Linked from the author guide
(P07-T02) and used for manual GA verification of the P04 install
path. Initial scope is **macOS only** (x86_64 + aarch64); Linux
support is tracked inside the example repo's own `PROJECTS.md` as
its P02 and is out of scope here.

**Out of Scope**
- Script/git kind (see P07a).
- Local-only example (see P07c).
- Linux assets (tracked in the example repo's `PROJECTS.md` P02).
- Building and publishing via `flox/flox-extension-precompile`
  (parallel workstream; P07b builds in its own CI).
- Use as a fixture for `cli/tests/extension.bats` — those tests use
  local fixtures; this repo is for users.
- Explicit `[extension.binary].platforms` map — the resolver
  priority #1 slot is reserved but not implemented in flox today
  (`github.rs:744`). Install uses priority #2 (`asset` template).
- Checksum verification of downloaded assets (v1 records but does
  not enforce; deferred to v3 per P04 scope).
- Telemetry in the example repo (tracks P07-T04, deferred).

### Tests & Tasks

- [x] [P07b-T01] Create the local working tree at
      `~/c/flox_repos/flox-hello-binary` and `git init -b main`.
- [x] [P07b-T02] Add `LICENSE` (Apache-2.0, matching flox) and an
      initial commit on `main`.
- [x] [P07b-T03] Author a minimal `flox-hello-binary` source
      program in Rust (single `main.rs` + `Cargo.toml`) that prints
      `Hello from $FLOX_EXTENSION_NAME $FLOX_EXTENSION_VERSION`
      and echoes any positional args. (`FLOX_EXTENSION_VERSION`
      already carries the `v` prefix, so the literal `v` from the
      original P07a template was dropped.)
- [x] [P07b-T04] Add `flox-extension.toml` with an `[extension]`
      table: `name = "hello-binary"`, a one-line `description`,
      and no `[environment]` block so the default `Inherit` mode
      applies. `[extension.binary]` carries `source =
      "github-release"` and `asset = "flox-hello-binary-{os}-
      {arch}.tar.gz"`. Assets are named `flox-hello-binary-macos-
      {aarch64,x86_64}.tar.gz` so the template renders exactly at
      runtime (`std::env::consts::OS` is `"macos"` on darwin).
- [x] [P07b-T05] Add `README.md`: install / upgrade / remove
      examples, platform matrix, and links to the flox author
      guide and the script-kind sibling repo.
- [x] [P07b-T06] Add `.github/workflows/release.yml`: on `v*`
      tag push, build `x86_64-apple-darwin` on `macos-13` and
      `aarch64-apple-darwin` on `macos-14`, archive each as
      `flox-hello-binary-macos-<arch>.tar.gz`, upload to a GitHub
      release via `gh release create`.
- [x] [P07b-T07] Add `.github/workflows/ci.yml`: `cargo fmt
      --check`, `cargo clippy --all-targets -- -D warnings`,
      `cargo test --all-targets` on `macos-14`; Python `tomllib`
      parse of `flox-extension.toml` on `ubuntu-latest`.
- [x] [P07b-T08] Create the **public** GitHub repo and push via
      `gh repo create flox/flox-hello-binary --public
      --source=. --remote=origin --push`. Verified public:
      `gh repo view … --json visibility` → `"PUBLIC"`.
- [x] [P07b-T09] Tag `v0.1.0` on the initial commit and push;
      release workflow builds and uploads the two macOS assets.
- [x] [P07b-T10] Apply the `flox-extension` GitHub topic via
      `gh repo edit flox/flox-hello-binary --add-topic
      flox-extension`. Verified: `repositoryTopics` includes
      `flox-extension`.
- [x] [P07b-T11] Add an example-repo-native `PROJECTS.md` that
      tracks this milestone as P01 and records cross-platform
      (Linux) compilation as P02, so future work has a home.
- [x] [P07b-T12] In **this** repo, edit
      `docs/extensions/author-guide.md` to point the binary-kind
      example link at `flox/flox-hello-binary` and note the
      macOS-only initial scope.
- [x] [P07b-TS01] Example repo CI: `cargo fmt --check`,
      `cargo clippy -- -D warnings`, and `cargo test` pass on
      `macos-14`.
- [x] [P07b-TS02] Example repo CI: `flox-extension.toml` parses
      as valid TOML and carries `[extension] name = "hello-
      binary"` + `[extension.binary] source` + `asset` with
      `{os}` / `{arch}` placeholders.
- [x] [P07b-TS03] Example repo release workflow: on `v0.1.0`
      tag, both macOS archives are built and uploaded. Verified
      via `gh release view v0.1.0 --json assets`.

### Deliverable

`flox extension install flox/flox-hello-binary` resolves
`v0.1.0`, downloads the host-matching macOS asset, records its
SHA-256 in `state.toml`, extracts the executable, and `flox
hello-binary` runs end-to-end against a GA flox binary on
macOS.

### Automated Verification

- Example repo CI is green on `main` (P07b-TS01, P07b-TS02).
- `v0.1.0` release carries the two macOS platform assets
  (P07b-TS03).

### Manual Verification

- Install on macOS arm64 and macOS x86_64 with a GA flox
  binary; confirm correct asset selection and that `flox
  hello-binary world` prints `Hello from hello-binary v0.1.0 /
  args: world`.
- On an Apple Silicon host, cut a release without the
  `aarch64` asset to confirm the `darwin-aarch64 →
  darwin-x86_64` Rosetta fallback (pairs with the P04 manual
  check).
- `flox extension upgrade hello-binary` is a no-op immediately
  after install, and picks up a bumped `v0.1.1` tag after one
  is cut.

### Exit Criterion

Repo is public at `github.com/flox/flox-hello-binary`, tagged
`v0.1.0` with both macOS platform assets attached, carries the
`flox-extension` topic, is linked from the author guide, and
installs cleanly via `flox extension install
flox/flox-hello-binary` on macOS x86_64 and macOS aarch64.
Cross-platform (Linux) compilation is tracked in the example
repo's `PROJECTS.md` P02.

---

## [x] Project P07c: flox-hello-local example repo (v1.0.0)

**Goal**: Publish `flox/flox-hello-local`, a template repo
intended to be cloned locally and installed via
`flox extension install --from-path`. Demonstrates the P02 local
dev loop. Linked from the author guide (P07-T02) and used for
manual GA verification of the P02 local-authoring path.

**Out of Scope**
- Remote clone-based install (covered by P07a).
- Binary assets (covered by P07b).
- Scaffolding automation (`flox extension create`, deferred to P08).
- Use as a fixture for `cli/tests/extension.bats` — those tests use
  local fixtures; this repo is for users.
- Telemetry in the example repo (tracks P07-T04, deferred).
- `flox extension upgrade` examples: unsupported for local-kind
  installs (returns `UpgradeError::LocalNotSupported`); the README
  documents the `--force` reinstall iterate loop instead.

### Tests & Tasks

- [x] [P07c-T01] Create the local working tree at
      `~/c/flox_repos/flox-hello-local` (`mkdir -p` the parent
      first) and `git init -b main` inside it.
- [x] [P07c-T02] Add `LICENSE` (Apache-2.0, matching flox) and an
      initial commit on `main`.
- [x] [P07c-T03] Add `flox-extension.toml` with an `[extension]`
      table: `name = "hello-local"` (matches the derived
      extension name from the `flox-hello-local` repo), a
      one-line `description`, no `[extension.binary]` stanza (so
      kind resolves to `script`), and **no** `[environment]`
      block — so the default `Inherit` mode applies.
- [x] [P07c-T04] Add executable `flox-hello-local` (bash,
      `#!/usr/bin/env bash`, `set -euo pipefail`) that prints
      `Hello from $FLOX_EXTENSION_NAME v$FLOX_EXTENSION_VERSION`
      and echoes any positional args. `chmod +x`. Exercises the
      P06-T06 bookkeeping env vars.
- [x] [P07c-T05] Add `README.md`: one-line description, a
      walkthrough for `git clone && flox extension install
      --from-path ./flox-hello-local`, iterate (`--force`
      reinstall) / remove command examples — **no** upgrade
      example, since `upgrade` is unsupported for local-kind —
      a link back to the flox author guide
      (`docs/extensions/author-guide.md`), and a note that this
      repo is the canonical P02 local-authoring reference.
- [x] [P07c-T06] Add `.github/workflows/ci.yml`: `shellcheck
      flox-hello-local` + a TOML-parse sanity check on
      `flox-extension.toml` (tiny Python `tomllib` script is
      fine). No flox-binary step.
- [x] [P07c-T07] Create the **public** GitHub repo and push via
      `gh repo create flox/flox-hello-local --public
      --source=. --remote=origin --push` from inside
      `~/c/flox_repos/flox-hello-local`. Verify public visibility
      with `gh repo view flox/flox-hello-local --json
      visibility` → `"PUBLIC"`.
- [x] [P07c-T08] Tag `v0.1.0` on the initial commit and push the
      tag (`git tag v0.1.0 && git push origin v0.1.0`). Parity
      with P07a/P07b release cadence; local install does not
      consume tags, so the tag is not referenced from the
      README.
- [x] [P07c-T09] Apply the `flox-extension` GitHub topic via
      `gh repo edit flox/flox-hello-local --add-topic
      flox-extension` so P08 `search` surfaces it. Verify with
      `gh repo view flox/flox-hello-local --json
      repositoryTopics`.
- [x] [P07c-T10] In **this** repo, edit
      `docs/extensions/author-guide.md` to cross-link the
      published example repo from the local-authoring /
      `--from-path` section.
- [x] [P07c-TS01] Example repo CI: `shellcheck flox-hello-local`
      passes.
- [x] [P07c-TS02] Example repo CI: `flox-extension.toml` parses
      as valid TOML and deserializes against the documented
      `AuthorManifest` shape (Python `tomllib` + assertions on
      the `[extension]` keys).

### Deliverable

`git clone https://github.com/flox/flox-hello-local &&
flox extension install --from-path ./flox-hello-local` succeeds and
`flox hello-local` runs.

### Automated Verification

- Example repo CI is green on `main` (P07c-TS01, P07c-TS02).
- `gh repo view flox/flox-hello-local --json
  visibility,repositoryTopics` shows `"PUBLIC"` and the
  `flox-extension` topic.

### Manual Verification

- Fresh clone + local install on Linux and macOS; confirm the README
  steps reproduce cleanly and `flox hello-local` prints the
  bookkeeping-env-var greeting.

### Exit Criterion

Repo is public at `github.com/flox/flox-hello-local`, tagged
`v0.1.0`, carries the `flox-extension` topic, is linked from the
author guide's local-authoring section, and the README walkthrough
reproduces cleanly.

---

## Cross-cutting concerns

Items that don't belong to a single project but are binding for the
subsystem as a whole.

- **Windows support.** Matches flox's own platform matrix; document as a
  decision, not a task. Any Windows-specific issues surfaced during
  P01–P07 are tracked here, not added to prior projects. (Research doc
  §2.11 #1.)
- **Git transport.** Shell out through the existing `GitCommandProvider`.
  No `git2` or `gix` dependency is added at any stage of v1.
- **Feature flag.** `Features::extensions` (default `false` until P07) is
  the single source of truth. `FLOX_FEATURES_EXTENSIONS=0/1` is the
  env-var override, consistent with other `FLOX_FEATURES_*` flags.
- **v2/v3 re-plan cadence.** After v1 GA (end of P07), run a fresh
  brainstorming/spec pass for P08, then a separate one for P09, then
  P10. Do not start P08 tasks without that pass.
- **Research-doc §2.11 open items.** Revisit each of `#1 Windows`,
  `#3 git transport`, `#4 archive formats`, `#6 activate re-entrancy`,
  `#7 telemetry` during P01–P07 and resolve them inside the relevant
  project rather than in a separate workstream.

---

## [x] Project P08: Extension discovery — search (v1.1.0)

**Goal**: Give users a way to find `flox-extension`-topic GitHub repos
from the CLI. Single-page `GET /search/repositories?q=topic:flox-
extension+archived:false+<query>+user:<owner>` against `GitHubSource`;
mark already-installed rows with `✓`; opportunistically honor
`GH_TOKEN` / `GITHUB_TOKEN` for a 10→30 req/min rate-limit bump.

The decision to use the GitHub `flox-extension` topic alone (not a
FloxHub-native index) is locked in for v1.1.0; FloxHub as a source
lands in P09 alongside the rest of FloxHub work.

**Out of Scope**
- Pagination (`page=N`, `per_page > 100`).
- FloxHub-native extension index (P09).
- `flox extension create` scaffolding (P09).
- Shell completion (P09).
- `flox extension browse` TUI (P10).
- `--json` output, response caching, custom/multiple topics (P10).

### Tests & Tasks

- [x] [P08-T01] In `cli/flox-rust-sdk/src/providers/extensions/github.rs`,
      add `SearchQuery { query: Option<String>, owner: Option<String>,
      limit: u8, sort: SearchSort }` and `SearchSort { Stars, Updated }`.
      Clamp `limit` to `1..=100` at construction.
- [x] [P08-T02] Same file: add `#[derive(Deserialize)]` response types
      `SearchResponse { total_count, incomplete_results, items }`,
      `SearchItem { full_name, owner, name, stargazers_count,
      description, archived, html_url }`, and `SearchOwner { login }`.
      Use `#[serde(default)]` on optional scalar fields so minimal
      fixtures still deserialize.
- [x] [P08-T03] Same file: extend `GitHubError` with
      `RateLimited { status: u16 }` (mapped from 403/429) and
      `AuthFailed { status: u16 }` (mapped from 401). Do not collapse
      into the existing `HttpStatus` variant — the user-facing message
      on `RateLimited` should hint at setting `GH_TOKEN` /
      `GITHUB_TOKEN`.
- [x] [P08-T04] Same file: add a private `auth_token_from_env()` that
      reads `GH_TOKEN` then `GITHUB_TOKEN` (first non-empty wins). Add
      an `auth_token: Option<String>` field on `GitHubSource`; populate
      in `from_env`. Add a `with_auth_token(self, Option<String>)`
      builder for tests. Keep the existing `new(client, base_url)`
      signature unchanged (defaults `auth_token = None`).
- [x] [P08-T05] Same file: implement `GitHubSource::search_repos(&self,
      q: &SearchQuery) -> Result<SearchResponse, GitHubError>`. Compose
      the `q` parameter by space-joining `topic:flox-extension`,
      `archived:false`, `<query?>`, `user:<owner?>` (no trailing spaces
      or empty tokens). Attach `per_page=<limit>`, `sort=<stars|updated>`,
      `order=desc`. Include `Authorization: Bearer <token>` when
      `auth_token` is set. Map 401→`AuthFailed`, 403/429→`RateLimited`,
      404→`NotFound`, other non-2xx→`HttpStatus`.
- [x] [P08-T06] In `cli/flox-rust-sdk/src/providers/extensions/manager.rs`,
      add `SearchError { Github(GitHubError), List(ListError) }` and
      `SearchRow { full_name: String, stars: u64, description:
      Option<String>, installed: bool }`. Implement `pub async fn
      search(flox: &Flox, q: &SearchQuery) -> Result<(Vec<SearchRow>,
      bool /* incomplete_results */), SearchError>`: call
      `GitHubSource::from_env().search_repos(q)`, call
      `list(flox)` to build a `HashSet<String>` of `"<owner>/<repo>"`
      (skip `kind == "local"`), mark rows. Re-export `search`,
      `SearchError`, `SearchQuery`, `SearchSort`, `SearchRow` from
      `mod.rs`.
- [x] [P08-T07] Create `cli/flox/src/commands/extension/search.rs` with
      a `#[derive(Debug, Bpaf, Clone)] pub struct Search { query:
      Option<String>, owner: Option<String>, limit: u8, sort: SortArg }`
      and a CLI-level `SortArg { Stars, Updated }`. Defaults: `limit =
      30`, `sort = Stars`. `handle(self, flox: Flox) -> Result<()>`
      follows the `list.rs` template: feature-flag early return,
      `subcommand_metric!("extension::search")`,
      `#[instrument(name = "extension::search", skip_all)]`, call
      `manager::search`, format the table, print the
      `incomplete_results` warning if set.
- [x] [P08-T08] In `cli/flox/src/commands/extension/mod.rs`: add
      `mod search;`, add a `Search(#[bpaf(external(search::search))]
      search::Search)` variant to `ExtensionCommands` between `Remove`
      and `Upgrade`, and add the matching
      `ExtensionCommands::Search(args) => args.handle(flox).await`
      match arm.
- [x] [P08-T09] Inline table helper in `search.rs` matching `list.rs`
      style: header `"{:<2}  {:<40}  {:>6}  {}"` over `" "`,
      `"OWNER/REPO"`, `"STARS"`, `"DESCRIPTION"`; row prefix `"✓ "`
      when `installed`, `"  "` otherwise. Truncate description to
      60 chars with an ellipsis. No new formatter crate.
- [x] [P08-T10] `incomplete_results` handling: after printing the
      table, if true, `eprintln!("warning: github reported incomplete
      results; re-run with a narrower query")`. Do not fail.
- [x] [P08-TS01] Unit (`github.rs`): query-string composition. Given
      `query = Some("hello")`, `owner = Some("acme")`, `limit = 25`,
      `sort = Stars`, an `httpmock` server matches
      `q=topic:flox-extension archived:false hello user:acme`,
      `per_page=25`, `sort=stars`, `order=desc`.
- [x] [P08-TS02] Unit: empty `query` + empty `owner` →
      `q=topic:flox-extension archived:false` (no trailing/empty
      tokens).
- [x] [P08-TS03] Unit: canned `/search/repositories` response with
      `incomplete_results: true`, `total_count: 2`, two items (one with
      `description: null`) deserializes.
- [x] [P08-TS04] Unit: mock returning `401` maps to
      `GitHubError::AuthFailed`; `403` and `429` both map to
      `GitHubError::RateLimited`; `500` maps to `GitHubError::HttpStatus`.
- [x] [P08-TS05] Unit: with an auth token set via `with_auth_token`,
      `search_repos` sends `Authorization: Bearer <token>`; without one,
      no Authorization header is sent. Uses the builder (not env
      mutation) to avoid Rust 2024 `unsafe { set_var }` and cross-test
      races.
- [x] [P08-TS06] Integ bats (`cli/tests/extension.bats`): extend
      `_setup_github_fixture`'s Python `http.server` with a
      `/search/repositories` route returning two items; install
      `owner/flox-hello` first (reusing the existing fixture install
      path), then run `flox extension search hello`. Assert stdout
      contains `✓ owner/flox-hello` and an unchecked row for the
      other item; exit 0.
- [x] [P08-TS07] Integ bats: fixture returning `incomplete_results:
      true` emits `warning: github reported incomplete results` on
      stderr; exit still 0.

### Deliverable

```bash
$ FLOX_FEATURES_EXTENSIONS=1 flox extension search hello
    OWNER/REPO                                STARS  DESCRIPTION
✓   flox-examples/flox-hello-script              42  canonical script-kind reference extension
    acme/flox-hello-deploy                       18  deploy helper that prints 'hello <env>'
    cortex/flox-hello-widget                      7  -

$ FLOX_FEATURES_EXTENSIONS=1 flox extension search --owner flox-examples --sort updated --limit 5
    OWNER/REPO                                STARS  DESCRIPTION
✓   flox-examples/flox-hello-script              42  canonical script-kind reference extension
    flox-examples/flox-hello-binary              31  canonical binary-kind reference extension
    flox-examples/flox-tool                      12  example multi-platform extension
```

### Automated Verification

- `just build-cli` succeeds.
- `just unit-tests` passes P08-TS01 through P08-TS05.
- `just integ-tests extension.bats` passes P08-TS06 and P08-TS07.

### Manual Verification

- Unauthenticated: `FLOX_FEATURES_EXTENSIONS=1 flox extension search
  hello` against live `api.github.com`; expect a hit list.
- Authenticated: `GH_TOKEN=$(gh auth token) FLOX_FEATURES_EXTENSIONS=1
  flox extension search --limit 100` runs without rate-limit errors
  under repeated invocations.
- Installed flag: install a known extension, re-run `search`, confirm
  the row shows `✓`.
- Auth error path: `GH_TOKEN=invalid flox extension search x` surfaces
  a message mentioning `GH_TOKEN` / `GITHUB_TOKEN`.

### Exit Criterion

`flox extension search <query>` returns GitHub repos tagged
`flox-extension`, marks already-installed ones with `✓`, and honors
`GH_TOKEN` / `GITHUB_TOKEN` for rate-limit relief.

---

## [ ] Project P09: Authoring and FloxHub source (v2)

> **Needs more detail before work starts — this project requires a
> separate brainstorming and spec pass. Goal and scope are listed for
> roadmap visibility only; no tasks are yet defined.**

**Goal** (from research doc §1.6):

- `flox extension create [--kind script|precompiled]`
- `FloxHubSource` as a second `ExtensionSource` implementation;
  `floxhub:org/name` install URIs.
- Shell completion (bpaf already supports generation; wire it in).

### Known open questions (must be resolved before task planning)

- What does a FloxHub "extension" object look like? No such object type
  exists in FloxHub today. (Research doc §2.11 #5.)
- Does `create` scaffold a GitHub repo, a local directory, or both?
- Is a `flox/flox-extension-precompile` GitHub Action a v2 deliverable
  inside this repo, a parallel workstream, or left to third parties?

### Tests & Tasks

_No task breakdown yet. Re-plan after v1 GA and P08 brainstorming._

---

## [ ] Project P10: Richer UX and ecosystem (v3)

> **Needs more detail before work starts — this project requires a
> separate brainstorming and spec pass. Goal and scope are listed for
> roadmap visibility only; no tasks are yet defined.**

**Goal** (from research doc §1.6):

- `flox extension browse` TUI, mirroring `gh extension browse`.
- `flox extension exec <name> …` as a name-conflict escape hatch.
- Nested extension commands (e.g., `flox env my-extension`).
- Optional checksum / build-provenance verification of binary assets.
- Flox Catalog integration for package dependencies declared by
  extensions.

### Known open questions

- What TUI framework does the rest of flox use? If none, choosing one is
  its own decision.
- Nested-command semantics: how does the parser disambiguate
  `env my-extension` from a subcommand typo?
- Does Catalog integration mean extensions declare a whole Flox
  environment as a dependency, or just pin specific packages?

### Tests & Tasks

_No task breakdown yet. Re-plan after v1 GA, P08, and P09 brainstorming._

---

## [~] Project P11: Rebase onto current main; gate behind `features.beta` (v1.2.0)

**Goal**: Carry P01–P08 forward onto current `origin/main` and change how
the subsystem is gated: instead of adding a bespoke `Features::extensions`
flag defaulting to `true`, reuse the existing `features.beta` flag,
opt-in and off by default.

Two upstream changes since `0badcdf59` drove the rework:

1. `Features` moved from `cli/flox-rust-sdk/src/flox.rs` to a new crate at
   `cli/flox-core/src/features.rs`, and already carries a `beta: bool`
   field. No new field is needed, so `flox-core` is untouched.
2. A `beta` crate (`cli/beta`) and a `BetaCommands` group
   (`cli/flox/src/commands/beta.rs`) now exist, with a documented
   convention in `.claude/skills/adding-beta-subcommand`. Beta commands
   are hidden from `flox --help` and gated once in the `Commands::Beta`
   arm.

**Out of Scope**
- Promoting extensions out of beta (that is a separate move, per the skill).
- Telemetry (still deferred from P07).
- FloxHub source (P09) and richer UX (P10).

### Tests & Tasks

- [x] [P11-T01] Fast-forward local `main` to `origin/main` and create the
      `smorin/github-extension-prototype-v2` worktree from it.
- [x] [P11-T02] Port the 23 new files from
      `smorin/github-extension-prototype`, including the uncommitted
      `extension.bats` mirror test.
- [x] [P11-T03] Relocate the domain module from
      `cli/flox-rust-sdk/src/providers/extensions/` to
      `cli/beta/src/extensions/`, so `flox-rust-sdk` is unchanged. Only
      three `crate::` imports needed rewriting (`Flox`, and the git
      provider); dependencies moved to `cli/beta/Cargo.toml`.
- [x] [P11-T04] Register `extension` on `BetaCommands` with
      `command("extension")` and `hide`, keeping the invocation as
      `flox extension …` while hiding it from `flox --help`.
- [x] [P11-T05] Delete the per-handler `ensure_extensions_enabled()` gate
      from all five verbs. The `Commands::Beta` arm gates once; handlers
      must not re-check (per the skill).
- [x] [P11-T06] Replace the opt-out `extensions_enabled()` with the opt-in
      `beta_enabled_from_env()`, reading `FLOX_FEATURES_BETA`.
- [x] [P11-T07] Add `sha2` and `zip` to the workspace (`flate2` is now
      present upstream) and wire `cli/beta/Cargo.toml`.
- [x] [P11-T08] Add `python3` to `pkgs/flox-cli-tests/default.nix` for the
      bats fake-GitHub fixture.
- [ ] [P11-TS01] `cargo build -p flox` succeeds.
- [ ] [P11-TS02] `flox extension list` without beta fails with the standard
      beta message; with `FLOX_FEATURES_BETA=true` it runs.
- [ ] [P11-TS03] `flox --help` does not list `extension`.
- [ ] [P11-TS04] `just integ-tests extension.bats` passes with the
      converted gating.
- [ ] [P11-T09] Decide whether to port the `extension_drift_tests` module
      (~80 lines in `cli/flox/src/commands/mod.rs`) that keeps
      `RESERVED_COMMAND_NAMES` in sync with the parser. Guards a real
      invariant, but adds test code to a heavily reviewed file.

### Known limitation

`flox <name>` dispatch reads `FLOX_FEATURES_BETA` from the environment
directly, because it runs in `main()` before the config system loads.
`flox config --set features.beta true` therefore enables the
`flox extension …` subcommands but **not** dispatch. Documented in the
user guide; should be resolved before extensions leave beta.

### Automated Verification

- `cargo build -p flox` succeeds.
- `git status --porcelain cli/flox-rust-sdk/` is empty — the SDK is untouched.
- `grep -rn 'FLOX_FEATURES_EXTENSIONS' cli/ docs/` returns zero hits.
- `./target/debug/flox --help` does not mention `extension`.
