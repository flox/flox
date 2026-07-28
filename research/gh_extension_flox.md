# PRD & Technical Plan — Porting `gh extension` to `flox extension`

> **Status:** Draft for engineering review
> **Target repos:** [`github.com/cli/cli`](https://github.com/cli/cli) (Go, reference implementation) → [`github.com/flox/flox`](https://github.com/flox/flox) (Rust, target)
> **Scope:** v1 minimum viable port; roadmap through v3
> **Date:** April 2026

---

## Table of contents

1. Part 1 — PRD
2. Part 2 — Technical architecture
3. Part 3 — Side-by-side mapping (Go → Rust)
4. Part 4 — Phased implementation plan

A note on sourcing: `gh` behavior is grounded in `pkg/cmd/extension/command.go`, `pkg/cmd/extension/manager.go` (the `installBin`/`installGit`/`Dispatch` functions), and `pkg/extensions/extension.go` in `cli/cli`; the concrete on‑disk `manifest.yml` schema is corroborated by reports from running installs (`~/.local/share/gh/extensions/gh-<name>/manifest.yml` contains `owner`, `name`, `host`, `tag`, `ispinned`, `path`). Flox behavior is grounded in the public docs, release notes, and observed paths (`cli/flox/`, `cli/flox-rust-sdk/`, `cli/flox-activations/` workspace crates; `bpaf` proc‑macro style; the `$FLOX_ENV`, `$FLOX_ENV_CACHE`, `$FLOX_ENV_PROJECT`, `$FLOX_ENV_DESCRIPTION`, `$FLOX_PROMPT_ENVIRONMENTS` activation env vars; `flox activate -- <cmd>` which `exec()`s in the activated environment since v1.9). A handful of specific file/line references below are tagged `[verify]` where they are derived from third‑party mirrors or PR discussions rather than a direct read of `main` — they should be re‑verified during implementation.

---

# Part 1 — PRD

## 1.1 Problem statement

Flox users today have no first‑class way to extend the `flox` CLI with custom commands. Any "extend Flox" use case — org‑specific wrappers, domain workflows, integrations with third‑party systems that shouldn't live in `cli/cli` — must be distributed as shell aliases, standalone binaries on `PATH`, or custom Nix expressions. None of these compose with the Flox environment model, none have a discovery/update story, and none make it obvious to a user that they are invoking a Flox extension rather than a core command.

`gh` solved the equivalent problem in 2021 with `gh extension` — a tiny, opinionated package manager for `gh-<name>` repos that ships scripts, precompiled binaries, or git‑cloned sources. The model has held up: the gh‑extension topic on GitHub has hundreds of published extensions and a mature authoring toolchain. **Porting that model to Flox, while honoring what makes Flox different (activated environments), is the fastest route to a credible extension story.**

## 1.2 Motivation — why this is worth doing now

Three forces align. **First**, Flox has reached architectural maturity: the activation subsystem was rewritten in Rust in v1.9 and now `exec()`s the child process in‑place, which means a Flox extension can be a subprocess of an activation with the exact same PID semantics as any other activated command. **Second**, FloxHub is the obvious long‑term distribution surface but isn't ready to be the *only* one; GitHub is where the Flox community already is. **Third**, the `gh` design is proven, small (roughly 2k lines of production Go in `pkg/cmd/extension/`), and maps cleanly onto Rust primitives Flox already uses (`reqwest`, `toml`, `git` via shell‑out, `bpaf` subcommands).

## 1.3 Goals and non‑goals

**Goals (v1):**

- Install, list, remove, and upgrade third‑party extensions from GitHub.
- Support the three `gh` flavors: script, precompiled binary from a GitHub Release, and git‑cloned source.
- Dispatch unknown `flox <name>` invocations to `flox-<name>` executables.
- By default, the extension inherits the currently active Flox environment, if any.
- An extension's manifest may declare a specific Flox environment that must be activated for its invocation — this is the one piece of novelty relative to `gh`.
- Abstract the "source" of an extension so FloxHub can be added in v2 without breaking changes.

**Non‑goals (v1), stated explicitly so reviewers see them:**

- **No sandboxing.** An extension runs with the user's full privileges, exactly like `gh`. Trust is by code review, not by technical enforcement.
- **No signature verification.** Even though the release‑asset resolver records a checksum, we do not verify signatures. `gh`'s build‑provenance attestation is interesting but out of scope.
- **No curated registry.** Discovery in v1 is "go to GitHub and search the `flox-extension` topic." A registry UX would follow FloxHub integration.
- **No search, browse, create, or exec subcommands in v1.** These are deferred to v2/v3 (§1.6).
- **No binary re‑publish layer.** We pull release assets straight from GitHub.
- **No support for extensions overriding built‑in commands.** Conflicts are rejected at install time.

## 1.4 Target users and workflows

| User | Workflow |
|---|---|
| **Platform engineer at a company using Flox** | Writes `flox-deploy`, a bash script that activates the team's `prod-tools` FloxHub environment and runs a deploy. Publishes it as `acme-corp/flox-deploy`. Team runs `flox extension install acme-corp/flox-deploy`, then `flox deploy staging`. |
| **Open‑source contributor** | Writes `flox-ai` in Rust, precompiled via GitHub Actions. Users run `flox extension install someone/flox-ai`; binary is downloaded for their OS/arch. |
| **Solo dev** | Clones an extension repo, runs `flox extension install .` for local development, iterates, pushes a v0.1.0 tag, other users install by tag. |
| **CI job** | Runs `flox extension install --pin v1.2.3 org/flox-report` in a pipeline to guarantee reproducibility. |

## 1.5 User stories

1. *As a Flox user*, I can install an extension from GitHub with a single command and immediately run it as `flox <name>`.
2. *As a Flox user*, I can list my installed extensions with their versions and see which ones have updates.
3. *As a Flox user*, I can pin an extension to a specific release tag or commit.
4. *As a Flox user*, I expect `flox <extname>` inside an activated environment to see that environment's packages on `PATH`.
5. *As an extension author*, I can declare `environment = "org/tools"` in my manifest so the extension *always* runs with that environment activated, independently of what the user has active.
6. *As an extension author*, I can opt out of automatic activation (`environment.inherit = "none"`) for extensions that must not pick up arbitrary user state.

## 1.6 Feature scope by version

### v1 — minimum viable (this document's design target)

- `flox extension install <repo> [--pin <ref>] [--force]`
- `flox extension list` (alias `ls`)
- `flox extension remove <name>`
- `flox extension upgrade <name> | --all [--dry-run] [--force]`
- Dispatch of `flox <name> …` to `flox-<name>`
- Script, binary (from GitHub Release), and git extension kinds
- Per‑extension `manifest.toml` (authored) + `state.toml` (internal)
- Environment inheritance with manifest override
- Source abstraction: `GitHubSource` is the only implementation

### v2 — discovery, authoring, and FloxHub

- `flox extension search [query] [--owner] [--limit]`
- `flox extension create [--kind script|precompiled]`
- `FloxHubSource` as a second `ExtensionSource` implementation; `flox extension install floxhub:org/name` URIs
- Shell completion (`bpaf` already supports this)

### v3 — richer UX and ecosystem

- `flox extension browse` (TUI, mirrors `gh extension browse`)
- `flox extension exec <name> …` for name‑conflict resolution
- Nested extension commands (e.g., `flox env my-extension`) — the feature `gh`'s team called out as roadmap
- Optional build‑provenance / checksum verification
- Flox Catalog integration for package dependencies declared by extensions

## 1.7 User‑facing command surface (v1)

```shell
# Install from GitHub
flox extension install flox-examples/flox-deploy
flox extension install https://github.com/flox-examples/flox-deploy
flox extension install .                               # local repo
flox extension install --pin v1.2.3 acme/flox-report
flox extension install --force acme/flox-report        # overwrite existing

# List (also shows update availability)
flox extension list
# NAME         REPO                              VERSION   PINNED
# deploy       flox-examples/flox-deploy         v0.3.1
# report       acme/flox-report                  v1.2.3    yes

# Upgrade
flox extension upgrade deploy
flox extension upgrade --all
flox extension upgrade --all --dry-run

# Remove
flox extension remove deploy

# Dispatch (no `extension` subcommand needed)
flox deploy staging --region us-east-1
```

### Invocation example showing environment inheritance

```shell
$ flox activate -d ./myproj          # activates myproj env
flox [myproj] $ flox deploy staging  # deploy extension sees myproj's PATH
flox [myproj] $ exit
$ flox deploy staging                # no env, runs with plain user PATH
```

### Invocation example showing manifest override

If `flox-deploy`'s manifest declares `environment = "acme/prod-tools"`, then:

```shell
$ flox deploy staging
# → Flox activates acme/prod-tools in a subprocess, then execs flox-deploy
#   with staging as argv[1], inheriting the activated env's PATH/vars.
```

## 1.8 Extension‑author surface

An extension is a GitHub repository named `flox-<name>`. It must contain either:

1. **An executable `flox-<name>` at the repo root** (script case), OR
2. **A GitHub Release with assets named `flox-<name>-<os>-<arch>[.exe]`** (binary case), OR
3. **Neither — but an executable `flox-<name>` exists at the repo root of a specific tree-ish** (git‑clone case; equivalent to (1) without a release).

All three kinds may ship a top‑level `flox-extension.toml` (authored metadata; see §1.9) — optional for script/git, **required** for binary releases because the resolver uses it to know the asset file name template if non‑default.

**Distinction from `gh` for authors:** a Flox extension may declare its activation behavior in the manifest; a `gh` extension cannot.

### Publishing flow (mirrors `gh`)

1. Name the repo `flox-<name>`.
2. For binary: add a GitHub Actions workflow that builds per‑platform assets and attaches them to releases. (An official `flox/flox-extension-precompile` action, modeled on `cli/gh-extension-precompile`, is a v2 deliverable.)
3. Push a `v*` tag to publish.
4. Tag the repo with the GitHub topic `flox-extension` so it is discoverable (future search).

## 1.9 Manifest / metadata file spec

Two files, both TOML. TOML chosen over YAML for consistency with Flox's existing `manifest.toml`/`manifest.lock` and to keep the author‑facing surface uniform. **Alternative considered:** YAML, as `gh` uses (`manifest.yml` with `owner`, `name`, `host`, `tag`, `ispinned`, `path`). Rejected because (a) the rest of the Flox ecosystem is TOML, (b) Flox already depends on `toml`/`toml_edit`, and (c) `manifest.yml` in `gh` is an *internal* state file, not author‑facing — we split that role explicitly.

### Author‑facing: `flox-extension.toml` (committed in the extension repo)

```toml
# Schema version — bump on breaking changes.
schema = 1

[extension]
name = "deploy"                # required; must match repo suffix flox-<name>
description = "Deploy a Flox environment to a target."
version = "0.3.1"              # SemVer; used when no git tag is resolvable
license = "Apache-2.0"
homepage = "https://github.com/flox-examples/flox-deploy"

# Which extension kind. If absent, inferred: release-with-assets => "binary",
# repo with top-level executable => "script", otherwise => "git".
kind = "binary"                # "script" | "binary" | "git"

# Binary kind only: asset naming. Default template is
#   "flox-<name>-<os>-<arch><ext>"  where <ext> is ".exe" on Windows, "" elsewhere.
# Override if your release uses a different convention.
[extension.binary]
asset_template = "flox-{name}-{os}-{arch}{ext}"
# Optional per-platform explicit mapping (overrides template).
[extension.binary.platforms]
"linux-x86_64"   = "deploy-linux-amd64.tar.gz"
"darwin-aarch64" = "deploy-macos-arm64.tar.gz"

# Environment behavior. All fields optional; this block as a whole is optional.
[environment]
# "inherit" (default): use whatever env the user has activated.
# "none":    explicitly run outside any Flox env.
# "pinned":  activate the env named in `ref` for every invocation.
inherit = "inherit"
ref     = "acme/prod-tools"    # required if inherit = "pinned"
# When pinned, how to resolve drift if the user is ALSO in an env:
# "override" (default) | "layer" (future) | "error"
on_active = "override"
```

### Internal: `state.toml` (written by the manager alongside the installed extension)

```toml
# Managed by Flox. Do not edit by hand.
schema = 1
name     = "deploy"
kind     = "binary"
source   = "github"
owner    = "flox-examples"
repo     = "flox-deploy"
host     = "github.com"            # supports GHE later
tag      = "v0.3.1"                # for binary: release tag; for git/script: ref
commit   = "a1b2c3d4..."           # for git/script: resolved commit SHA
pinned   = false                   # true if installed with --pin
asset_sha256 = "fe14...cafe"       # for binary; recorded but not verified in v1
installed_at = "2026-04-17T10:11:12Z"
path         = "/home/me/.local/share/flox/extensions/flox-deploy/flox-deploy"
```

This mirrors `gh`'s `manifest.yml` fields (`owner`, `name`, `host`, `tag`, `ispinned`, `path`) and adds `kind`, `commit`, `source`, and `asset_sha256` to cover the three kinds under a single struct and prepare for FloxHub.

## 1.10 Dispatch semantics

When the user types `flox foo bar baz` and `foo` is not a known top‑level subcommand:

1. The `bpaf` parser either fails or the top‑level enum has an `External(Vec<OsString>)` variant (see §2.5). We catch this before emitting the "unknown subcommand" error.
2. The manager looks for `flox-foo` first in the managed extensions directory (`$XDG_DATA_HOME/flox/extensions/flox-foo/flox-foo`), then falls back to `$PATH`. This mirrors `gh`'s behavior and allows users to `flox extension install .` a locally developed extension without polluting `$PATH`.
3. The child process is spawned with all remaining argv (`bar baz`) forwarded untouched.
4. Environment variables are set/inherited per §1.11.
5. Exit code of the extension is the exit code of `flox`.

If both `foo` is unknown *and* no `flox-foo` exists, we emit the standard `bpaf` "unknown command" error with a hint: `try 'flox extension install <repo>'`.

**Alternative considered:** pre‑scanning `$PATH` on every `flox` invocation to build the command list. Rejected: adds latency to every CLI run; `gh` does not do this; lookup on demand is correct.

## 1.11 Environment inheritance

The rule: **the extension's `manifest.environment` stanza, if present and of kind `pinned`, wins; otherwise the user's currently‑active Flox environment is inherited.**

Resolution algorithm on dispatch:

1. Read the extension's `flox-extension.toml` (cached from install).
2. Determine `mode`:
   - If `environment.inherit = "pinned"` → `mode = Pinned(ref)`.
   - Else if `$FLOX_ENV` is set (user has an active activation) → `mode = Inherit`.
   - Else `mode = None`.
3. Execute based on mode:
   - **Pinned:** run the equivalent of `flox activate -r <ref> -- flox-<name> <args...>`. Reuses existing activation plumbing; guarantees the extension runs with exactly the environment the author intended.
   - **Inherit:** spawn the child in‑process with `$FLOX_ENV`, `$FLOX_ENV_CACHE`, `$FLOX_ENV_PROJECT`, `$FLOX_ENV_DESCRIPTION`, `$FLOX_PROMPT_ENVIRONMENTS`, and the activated `$PATH` already inherited from the parent process. Nothing special to do — the child inherits the parent's env by default.
   - **None:** spawn the child with the user's baseline env (no Flox vars). We explicitly scrub any leaked `$FLOX_*` vars for predictability.
4. In every mode, we additionally set:
   - `FLOX_EXTENSION_NAME=<name>`
   - `FLOX_EXTENSION_VERSION=<version>`
   - `FLOX_EXTENSION_PATH=<install dir>`
   - `FLOX_BIN=<absolute path to current flox binary>` — so extensions can shell out to `flox` itself reliably without `$PATH` ambiguity. This is the analog of `gh`'s `GH_PATH`.

**Why `on_active = "override"` is the default when pinned:** an author who pins an environment does so because their extension needs it. Silently falling through to the user's env would be a footgun. `"error"` is the strict alternative and is supported. `"layer"` (compose the two envs) is explicitly deferred because Flox composition semantics are still evolving.

## 1.12 Compatibility and migration

- **No breaking changes** to existing `flox` CLI surface. All new work lives under `flox extension …` plus an external‑subcommand fallback that only triggers on otherwise‑unknown commands.
- **Namespace reservation:** the `extension` keyword is new. Users who have a local `flox-extension` binary on `$PATH` will no longer dispatch to it, because `extension` is now a core subcommand. This is a deliberate, documented incompatibility; searching the Flox forum shows no current usage of this name.
- **v2 FloxHub migration:** `state.toml`'s `source = "github"` field is the discriminant. When FloxHub support lands, `source = "floxhub"` extensions will use a parallel resolver; no state migration is required for existing GitHub‑installed extensions.

## 1.13 Success metrics

- **Adoption:** ≥ 20 community‑published `flox-extension`‑topic repos within 6 months of v1 GA.
- **Reliability:** < 1% install failure rate, measured by anonymous telemetry on `flox extension install` exit codes (respecting opt‑out).
- **Performance budget:** extension dispatch overhead ≤ 50 ms on top of the extension's own startup (measured: time from `flox foo` invocation to extension's `main`).
- **Time‑to‑first‑extension:** a new user can publish a working script extension in under 15 minutes following the docs.

## 1.14 Explicit non‑goals recap

| Non‑goal | Rationale |
|---|---|
| Sandboxing | Matches `gh`. Extensions are trusted code. |
| Signature/attestation verification | Deferred; `asset_sha256` is recorded for a future flag. |
| Curated registry | The GitHub topic is sufficient v1 discovery. |
| Overriding core commands | Collision rejected at install; use `flox extension exec` (v3) to escape. |
| Bundling the Flox env into the extension artifact | Extensions are small; environments are big and already distributed via FloxHub. |

---

# Part 2 — Technical architecture

## 2.1 High‑level architecture

```
                 ┌────────────────────────────────────────────┐
 user types  ──► │  flox  (cli/flox/src/main.rs)              │
                 │    bpaf top-level parser                   │
                 └────┬─────────────────────────────┬─────────┘
                      │ matches known subcmd        │ unknown token
                      ▼                             ▼
         ┌────────────────────────┐      ┌─────────────────────────────┐
         │ Commands::Extension(…) │      │ Commands::External(argv)    │
         │  install/list/remove/  │      │   ExtensionManager::find()  │
         │  upgrade               │      │   + Dispatcher::exec()      │
         └──────────┬─────────────┘      └──────┬──────────────────────┘
                    │                            │
                    ▼                            │
         ┌────────────────────────┐              │
         │  ExtensionManager      │◄─────────────┘
         │   (trait)              │
         │                        │
         │  dyn ExtensionSource ──┼──► GitHubSource  (v1)
         │  FS layout             │    FloxHubSource (v2)
         │  state.toml r/w        │
         └──────────┬─────────────┘
                    │
         ┌──────────▼─────────────────────────────────────────┐
         │ Filesystem: $XDG_DATA_HOME/flox/extensions/        │
         │   flox-deploy/                                     │
         │     flox-deploy        (executable, symlink, .git) │
         │     state.toml                                     │
         │     flox-extension.toml  (copied from repo)        │
         └────────────────────────────────────────────────────┘
```

Dispatch path in detail:

```
        flox foo bar baz
              │
              ▼
     bpaf parser ─── recognized? ──yes──► usual path
              │no
              ▼
     External(["foo","bar","baz"])
              │
              ▼
     ExtensionManager::find("foo")
              │       │
           Some(ext)  None → error + "try flox extension install"
              │
              ▼
     activation_mode = resolve(ext.manifest, $FLOX_ENV)
              │
     ┌────────┴────────┬─────────────┐
  Pinned(ref)        Inherit        None
     │                 │              │
     ▼                 ▼              ▼
  spawn:          spawn directly   spawn (scrubbed env)
  flox activate   with inherited
  -r <ref>        $FLOX_* vars
  -- flox-foo …
```

## 2.2 Crate / module layout

Flox is a Cargo workspace with (among others) `cli/flox/` (the binary crate), `cli/flox-rust-sdk/` (the library), and `cli/flox-activations/` (the activation logic extracted in v1.9). We place new code accordingly:

```
cli/
├── flox/
│   └── src/
│       ├── main.rs
│       └── commands/
│           ├── mod.rs                    ← register Extension variant, External fallback
│           └── extension/                ← NEW subcommand module
│               ├── mod.rs                ← bpaf enum + handler entrypoint
│               ├── install.rs
│               ├── list.rs
│               ├── remove.rs
│               └── upgrade.rs
└── flox-rust-sdk/
    └── src/
        └── providers/
            └── extensions/               ← NEW module tree
                ├── mod.rs                ← public re-exports
                ├── manager.rs            ← ExtensionManager trait + impl
                ├── extension.rs          ← Extension struct + Kind enum
                ├── manifest.rs           ← AuthorManifest + InstalledState
                ├── dispatch.rs           ← find + spawn logic
                ├── layout.rs             ← on-disk paths
                ├── source.rs             ← ExtensionSource trait
                ├── github.rs             ← GitHubSource impl
                └── floxhub.rs            ← stub in v1; impl in v2
```

**Why split between `cli/flox/` and `cli/flox-rust-sdk/`:** the pattern the repo already uses. Command parsing and user‑facing error presentation live in the binary crate; the domain logic (manager, traits, FS layout) lives in the SDK so it is testable without a `bpaf` context and so FloxHub or the VS Code extension can reuse it later. This is analogous to `gh` keeping the `pkg/extensions/` interface separate from the `pkg/cmd/extension/` command surface.

## 2.3 Core trait sketches

All code uses `async fn` where Flox's reqwest calls already are `async`, and uses `thiserror` for a typed error enum (Flox's convention) with `anyhow` reserved for binary‑level glue. Where `impl Trait` in return position would be clearer than a boxed future, it is preferred — but the trait object needs to be dyn‑compatible, so the canonical trait uses `async_trait` for now (matching what Flox does for other provider traits).

```rust
// cli/flox-rust-sdk/src/providers/extensions/extension.rs

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

/// One of three flavors, mirroring gh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    Script,
    Binary,
    Git,
}

/// A fully-installed extension on disk.
#[derive(Debug, Clone)]
pub struct Extension {
    pub name: String,            // "deploy" (no "flox-" prefix)
    pub kind: ExtensionKind,
    pub owner: String,           // "flox-examples"
    pub repo: String,            // "flox-deploy"
    pub host: String,            // "github.com"
    pub tag: Option<String>,     // release tag if known
    pub commit: Option<String>,  // git SHA for script/git kinds
    pub pinned: bool,
    pub install_dir: PathBuf,    // .../extensions/flox-deploy/
    pub executable: PathBuf,     // .../flox-deploy
    pub manifest: AuthorManifest, // parsed flox-extension.toml
}
```

```rust
// cli/flox-rust-sdk/src/providers/extensions/manager.rs

use async_trait::async_trait;
use std::path::PathBuf;

#[async_trait]
pub trait ExtensionManager: Send + Sync {
    /// Enumerate installed extensions.
    async fn list(&self) -> Result<Vec<Extension>, ExtensionError>;

    /// Install from a source spec, optionally pinned.
    async fn install(
        &self,
        spec: &ExtensionSpec,
        opts: InstallOptions,
    ) -> Result<Extension, ExtensionError>;

    /// Install from a local path (used by `flox extension install .`).
    async fn install_local(&self, path: &std::path::Path)
        -> Result<Extension, ExtensionError>;

    /// Upgrade one (name) or all (None).
    async fn upgrade(
        &self,
        name: Option<&str>,
        opts: UpgradeOptions,
    ) -> Result<Vec<UpgradeResult>, ExtensionError>;

    /// Remove by name.
    async fn remove(&self, name: &str) -> Result<(), ExtensionError>;

    /// Lookup for dispatch. Returns None if not an installed extension; callers
    /// then fall back to $PATH.
    async fn find(&self, name: &str) -> Result<Option<Extension>, ExtensionError>;
}

#[derive(Debug, Clone)]
pub struct ExtensionSpec {
    pub source: SourceKind,                 // GitHub in v1
    pub ident:  SourceIdent,                // owner/repo or FloxHub ref
}

#[derive(Debug, Clone)]
pub enum SourceKind { GitHub, FloxHub }     // FloxHub is a type marker in v1

#[derive(Debug, Clone)]
pub struct SourceIdent { pub owner: String, pub repo: String, pub host: String }

#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    pub pin:   Option<String>,      // tag or commit
    pub force: bool,                // overwrite existing
}

#[derive(Debug, Clone, Default)]
pub struct UpgradeOptions { pub force: bool, pub dry_run: bool }

#[derive(Debug, Clone)]
pub struct UpgradeResult {
    pub name: String,
    pub from: String,
    pub to:   String,
    pub status: UpgradeStatus,
}
#[derive(Debug, Clone)]
pub enum UpgradeStatus { Upgraded, AlreadyCurrent, Pinned, Failed(String) }
```

```rust
// cli/flox-rust-sdk/src/providers/extensions/source.rs

#[async_trait]
pub trait ExtensionSource: Send + Sync {
    /// Identify latest tag/commit. Returns (tag, commit).
    async fn resolve_latest(&self, id: &SourceIdent)
        -> Result<Resolved, ExtensionError>;

    /// Resolve a user-provided pin (tag or SHA prefix) to a concrete ref.
    async fn resolve_pin(&self, id: &SourceIdent, pin: &str)
        -> Result<Resolved, ExtensionError>;

    /// Is there a release with downloadable binary assets for (tag)?
    async fn list_release_assets(&self, id: &SourceIdent, tag: &str)
        -> Result<Vec<ReleaseAsset>, ExtensionError>;

    /// Download an asset to `dest`. Returns sha256 hex.
    async fn download_asset(
        &self, asset: &ReleaseAsset, dest: &std::path::Path,
    ) -> Result<String, ExtensionError>;

    /// Shallow-clone the repo at a specific ref to `dest`.
    async fn clone_repo(
        &self, id: &SourceIdent, reference: &str, dest: &std::path::Path,
    ) -> Result<(), ExtensionError>;
}

#[derive(Debug, Clone)]
pub struct Resolved { pub tag: Option<String>, pub commit: String }

#[derive(Debug, Clone)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
    pub size: u64,
    pub content_type: Option<String>,
}
```

```rust
// cli/flox-rust-sdk/src/providers/extensions/manifest.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthorManifest {
    pub schema: u32,
    pub extension: ExtensionMeta,
    #[serde(default)]
    pub environment: EnvironmentBehavior,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtensionMeta {
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub license: Option<String>,
    pub homepage: Option<String>,
    pub kind: Option<ExtensionKind>,          // inferred if absent
    #[serde(default)]
    pub binary: BinaryMeta,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct BinaryMeta {
    pub asset_template: Option<String>,        // default in code
    pub platforms: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct EnvironmentBehavior {
    #[serde(default = "default_inherit")]
    pub inherit: InheritMode,
    pub r#ref:   Option<String>,
    #[serde(default)]
    pub on_active: OnActive,
}
fn default_inherit() -> InheritMode { InheritMode::Inherit }

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InheritMode {
    #[default] Inherit,
    None,
    Pinned,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnActive { #[default] Override, Error }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstalledState {
    pub schema: u32,
    pub name: String,
    pub kind: ExtensionKind,
    pub source: String,
    pub owner: String,
    pub repo: String,
    pub host: String,
    pub tag: Option<String>,
    pub commit: Option<String>,
    pub pinned: bool,
    pub asset_sha256: Option<String>,
    pub installed_at: String,       // RFC 3339
    pub path: std::path::PathBuf,
}
```

```rust
// cli/flox-rust-sdk/src/providers/extensions/manager.rs (cont'd)

#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("extension '{0}' is already installed")]
    AlreadyInstalled(String),
    #[error("extension '{0}' not found")]
    NotInstalled(String),
    #[error("extension name '{0}' conflicts with a core flox command")]
    CommandConflict(String),
    #[error("no release asset matching platform {platform} for {owner}/{repo}")]
    NoMatchingAsset { owner: String, repo: String, platform: String },
    #[error("could not resolve release or ref for {0}/{1}")]
    ResolveFailed(String, String),
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    #[error("git operation failed: {0}")]
    Git(String),
}
```

## 2.4 On‑disk layout

We follow XDG and the paths Flox already uses (`~/.config/flox/` for config, `~/.local/share/flox/` for data, `~/.cache/flox/` for cache — consistent with the debug trace `flox/src/config/mod.rs: $FLOX_CONFIG_HOME not set, using "/root/.config/flox/"`).

```
$XDG_DATA_HOME/flox/extensions/            ← default root
├── .lock                                  ← flock for concurrent install/upgrade
├── index.toml                             ← optional cache: name → install_dir
└── flox-<name>/
    ├── flox-<name>                        ← executable (file | symlink | inside .git)
    ├── state.toml                         ← managed InstalledState
    ├── flox-extension.toml                ← copy of author manifest
    ├── .git/                              ← for git/script kinds only
    └── .checksum                          ← for binary kind: sha256 hex line
```

Per‑kind specifics:

- **Script (git‑clone):** `git clone --depth=1 [--branch <pin>]` into the install dir; executable is a relative path inside the clone. A git‑clone and a "script" in `gh`'s taxonomy are the same thing on disk; the distinction only matters for upgrade (re‑clone vs. re‑download).
- **Binary:** the release asset is downloaded directly as `flox-<name>` (or extracted, if the asset is a `.tar.gz`/`.zip`), `chmod +x`'d, and `sha256` recorded. No `.git/`.
- **Local (`install .`):** `state.toml`'s `path` points at a symlink into the user's working copy. `gh` does the same; it enables iterative development without reinstalling.

The `.lock` file uses `fs2::FileExt::try_lock_exclusive()` to serialize `install`/`upgrade`/`remove`. Read operations (`list`, `find`) are lock‑free since `state.toml` writes are atomic (temp‑file rename).

## 2.5 Dispatch integration with `bpaf`

Flox uses `bpaf` with the derive macro. The current top‑level command enum lives at `cli/flox/src/commands/mod.rs`. We add one enum variant for the managed subcommand plus an external fallback:

```rust
// cli/flox/src/commands/mod.rs (additions)

use bpaf::Bpaf;

#[derive(Debug, Clone, Bpaf)]
#[bpaf(options, version)]
pub enum Commands {
    // ... existing variants: Init, Install, Activate, Search, …

    /// Manage flox extensions
    #[bpaf(command)]
    Extension(#[bpaf(external(extension::extension))] extension::ExtensionCmd),

    /// Catch-all: an unrecognized first-positional + everything after it.
    /// Triggered only if no other variant matched.
    #[bpaf(command(".extension-dispatch"), hide)]   // placeholder
    External {
        #[bpaf(any::<std::ffi::OsString, _, _>("CMD", Some))]
        argv: Vec<std::ffi::OsString>,
    },
}
```

`bpaf` does not have a direct `clap`‑style `allow_external_subcommands` flag; the idiomatic approach is a fallback parser assembled after all named commands. The production pattern is to call `parse()` twice with `run_inner`: first with the full command set, and if it returns `ParseFailure::Stderr` on an unknown‑subcommand error, re‑parse with a parser that accepts the raw `argv` as `any::<OsString>(…).many()`. A thin helper in `main.rs`:

```rust
// cli/flox/src/main.rs

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();

    // 1. Try the normal parser.
    match commands::commands().run_inner(bpaf::Args::from(args.as_slice())) {
        Ok(cmd) => run(cmd),

        // 2. If bpaf didn't recognize the subcommand and the first arg isn't a
        //    flag, try dispatch.
        Err(bpaf::ParseFailure::Stderr(_)) if looks_like_extension(&args) => {
            dispatch_extension(args)
        }

        Err(err) => {
            err.print_mesage(80);
            ExitCode::from(2)
        }
    }
}

fn looks_like_extension(args: &[std::ffi::OsString]) -> bool {
    args.first()
        .and_then(|a| a.to_str())
        .map(|s| !s.starts_with('-'))
        .unwrap_or(false)
}
```

`dispatch_extension` calls `ExtensionManager::find(&args[0])`; if `Some(ext)`, it launches the child (see §2.7); if `None`, it prints the standard `bpaf` error, plus a `Try: flox extension install <owner>/flox-<name>` hint.

**Alternative considered:** adding a new top‑level positional `Commands::Unknown(Vec<OsString>)` with `bpaf::any` as the variant‑level parser and ordering it last. Rejected because `bpaf` evaluates alternatives left‑to‑right and greedy `any` swallows legitimate unknown‑flag errors, harming UX for typos. The two‑phase parse keeps built‑in UX intact.

## 2.6 Installation flow (per kind)

### Script / binary / git — common preamble

1. Parse the user's argument: `owner/repo`, full URL, or `.`.
2. Normalize: ensure repo name starts with `flox-`; extract `name` (suffix).
3. `checkValidExtension`: reject if name collides with a core `flox` subcommand (consult the `Commands` enum via a compile‑time list) or with an already‑installed extension (unless `--force`).
4. Create `$XDG_DATA_HOME/flox/extensions/flox-<name>/` under a staging directory (`flox-<name>.staging-<uuid>`) for atomic rename on success.

### Git/script kind (no release assets found)

1. `git clone --depth=1 --branch <pin_or_default> https://<host>/<owner>/<repo>.git <staging>`.
2. Verify `<staging>/flox-<name>` exists and is executable (or `chmod +x` it if not).
3. Copy `flox-extension.toml` to staging root if present in the clone.
4. Write `state.toml` with `kind = "script"` (or `"git"` if no entrypoint was found — we still allow a repo that uses `exec` to pick a language‑specific entrypoint).
5. `rename(staging → final)`.

### Binary kind (release asset exists)

1. Call `GitHubSource::resolve_latest` — hits `GET /repos/:owner/:repo/releases/latest` (or `/releases/tags/<pin>`).
2. Compute the current platform string: `format!("{os}-{arch}")` where `os ∈ {linux, darwin, windows}`, `arch ∈ {amd64 or x86_64, arm64 or aarch64}`. Emit both nomenclatures for matching because `gh`‑style assets use `amd64` but Rust's `std::env::consts::ARCH` returns `x86_64`.
3. Resolve the asset:
   - If `manifest.extension.binary.platforms[<platform>]` is set, use that exact name.
   - Else render `asset_template` (default `flox-{name}-{os}-{arch}{ext}`).
   - Else substring‑match release asset names for the platform string, mirroring `installBin` in `cli/cli` which simply walks the release assets and picks one whose name contains the platform; the `darwin-arm64` → `darwin-amd64` Rosetta fallback documented in `cli/cli#9592` is reproduced.
4. Download to staging, compute sha256, record in `.checksum`.
5. If asset is an archive (`.tar.gz`/`.zip`), extract and locate an executable named `flox-<name>`.
6. `chmod +x` and write `state.toml`.

### Local (`install .`)

1. Resolve to absolute path.
2. Symlink `final/flox-<name>` → `<abs>/flox-<name>`.
3. Copy `flox-extension.toml` in (not symlinked, so edits don't corrupt state until reinstall).
4. Write `state.toml` with `source = "local"`, `commit = <git rev-parse HEAD>` if the dir is a git repo, else omitted.

### Name‑conflict and already‑installed behavior

- If installed *with the same owner*, offer to upgrade (matches `gh`'s `alreadyInstalledError` path).
- If installed *with a different owner* and `--force`, delete the existing install dir and proceed.
- Otherwise emit `ExtensionError::AlreadyInstalled`.

## 2.7 Upgrade flow

`flox extension upgrade <name>` or `--all`:

1. Load `state.toml` for each target.
2. Skip if `pinned = true` (unless `--force`).
3. Per kind:
   - **Binary:** `resolve_latest`; if `tag == state.tag`, report `AlreadyCurrent`. Otherwise re‑run the install flow on a staging dir, then atomic rename.
   - **Git/script:** `git -C <dir> fetch --depth=1 origin <default_branch>` then `git reset --hard FETCH_HEAD`. Record the new commit SHA in `state.toml`.
   - **Local:** refuse (can't upgrade a symlink to your working tree); print a notice.
4. With `--dry-run`, run resolution only; emit a table of what *would* happen.

Version tracking is straightforward: `state.toml.tag` for binary, `state.toml.commit` for git/script. `flox extension list` displays `tag` when present, else 8‑char truncated `commit`, matching `gh`'s `displayExtensionVersion` logic.

## 2.8 Environment‑activation process model

The extension child process is spawned by `cli/flox-rust-sdk/src/providers/extensions/dispatch.rs`:

```rust
pub async fn spawn_extension(
    ext: &Extension,
    argv: &[std::ffi::OsString],
    flox: &Flox,                       // existing shared ctx
) -> Result<std::process::ExitCode, ExtensionError> {
    let mode = resolve_mode(&ext.manifest.environment, std::env::vars_os().into_iter());

    match mode {
        ActivationMode::Pinned(env_ref) => {
            // Delegate to existing activate machinery so we benefit from its
            // caching, watchdog, trust prompts, and exec() semantics.
            // Equivalent to: `flox activate -r <env_ref> -- flox-<name> <argv…>`
            activate_and_exec(flox, &env_ref, &ext.executable, argv).await
        }
        ActivationMode::Inherit => {
            // Just spawn with the parent's env. $FLOX_ENV et al. are already set.
            exec_child(&ext.executable, argv, std::env::vars_os()).await
        }
        ActivationMode::None => {
            let scrubbed = std::env::vars_os()
                .filter(|(k, _)| !k.to_string_lossy().starts_with("FLOX_"));
            exec_child(&ext.executable, argv, scrubbed).await
        }
    }
}
```

`activate_and_exec` calls into `cli/flox-activations/` (the crate extracted for the v1.9 rewrite). It should *not* re‑shell through `sh -c "flox activate …"`; it should use the same in‑process path as `flox activate -- <cmd>` so the child ends up with the same PID semantics as any other activated command (per the v1.9 release notes, `flox activate -- <cmd>` now `exec()`s).

In all three modes, we inject the bookkeeping vars:

```rust
cmd.env("FLOX_EXTENSION_NAME", &ext.name)
   .env("FLOX_EXTENSION_VERSION", ext.tag.as_deref().unwrap_or(""))
   .env("FLOX_EXTENSION_PATH", &ext.install_dir)
   .env("FLOX_BIN", std::env::current_exe()?);
```

## 2.9 Error types and user‑facing messages

`ExtensionError` is the crate‑internal type. At the CLI boundary the handler converts to the existing Flox error reporting convention (a `Display`‑oriented enum with ANSI coloring). Sample surface strings, modeled on `gh`'s:

| Condition | Message |
|---|---|
| Install conflicts with core command | `error: name 'activate' conflicts with a built-in flox command` |
| Install conflicts with installed ext | `error: flox-deploy is already installed (run with --force to overwrite)` |
| No matching asset | `error: no release asset matches 'linux-x86_64' for flox-examples/flox-deploy`<br>`hint: open an issue asking the maintainer to publish a linux-x86_64 build` |
| Extension executable missing | `error: extension 'deploy' has no executable at .../flox-deploy/flox-deploy` |
| Pinned, env trust declined | `error: extension 'deploy' requires the 'acme/prod-tools' environment; trust it with 'flox activate -r acme/prod-tools --trust' first` |
| Upgrade of pinned | `skipping 'deploy' (pinned to v1.2.3); pass --force to override` |

## 2.10 Testing strategy

Three tiers, matching Flox's existing conventions:

**Unit (inline `#[cfg(test)]` in each module):**
- `manifest.rs`: round‑trip TOML parsing and defaults.
- `layout.rs`: path composition across `$XDG_DATA_HOME` values, including when unset.
- `source.rs` / `github.rs`: asset‑resolution algorithm against canned release JSON fixtures (stored under `cli/flox-rust-sdk/tests/fixtures/github/releases/*.json`); use `wiremock` to stub the GitHub API. This is the analog of `manager_test.go` in `gh`.
- `dispatch.rs`: `resolve_mode` truth table across all `InheritMode` × `$FLOX_ENV` combinations.

**Integration (crate‑level `tests/`):**
- End‑to‑end install/list/remove/upgrade of a fixture script extension served from a local `tempfile`‑backed HTTP mock and a local bare git repo (`git init --bare` + `gix` or shell‑out).
- Dispatch integration test: install a "hello" extension whose `flox-hello` prints `$FLOX_EXTENSION_NAME`, spawn it, assert stdout.

**Bats (existing Flox convention):**
- `cli/flox/tests/extension.bats` with user‑facing scenarios: install `.`, install with `--pin`, upgrade `--all`, dispatch inside and outside an active env, and the pinned‑env override case.
- Run under CI with `flox build && flox activate -- bats …` like the rest of the suite.

Mocks: Flox uses hand‑rolled trait impls for test doubles rather than `mockall`. We follow suit — `ExtensionSource` is dyn‑compatible and a `TestSource` fixture lives in `#[cfg(test)]` modules.

## 2.11 Open questions and risks

1. **Windows.** `gh` supports Windows via `.exe` extension and symlink fallbacks. Flox v1 largely targets macOS/Linux with WSL. Confirm whether the extension system is Linux+macOS only in v1 or must also handle WSL properly. Reasonable default: same platform matrix as `flox` itself.
2. **Name‑conflict list.** The list of reserved top‑level command names must be kept in sync with `Commands`. Propose a compile‑time `const` slice plus a unit test that walks the `bpaf` help output to enforce it.
3. **Git transport.** Flox is not currently known to depend on `git2`/`gix`. Proposal: shell out to `/usr/bin/env git` (simpler, matches `gh`'s `git.Client`). Document `git` as a runtime dependency — it already effectively is.
4. **Archive formats.** `gh` expects raw binary assets; our binary kind also supports `.tar.gz`/`.zip`. Use `flate2` + `tar` + `zip` crates. Alternative: mandate raw binaries in v1 and defer archive support. Recommendation: support archives from day one; it dramatically simplifies authoring for non‑Go languages where a single‑file artifact is unusual.
5. **FloxHub extension fetch primitive.** FloxHub exposes `hub.flox.dev/...` endpoints for environments today; whether an "extension" object type exists there is a separate design question. We keep the `ExtensionSource` trait neutral so either a direct URL or a new FloxHub artifact type can back it.
6. **`flox activate` reentrancy when pinned.** If a user runs `flox deploy` *while already inside* the `acme/prod-tools` activation the extension pins, re‑activating should be a no‑op rather than layering. Flox's v1.9 rewrite added "concurrent activations of the same environment in different modes (run vs. dev) are now prevented"; we need a read‑through on the current behavior to confirm the idempotent path is covered.
7. **Telemetry.** Should `flox extension install` events be included in the existing metrics stream (the Flox debug trace references `Metrics collection disabled`)? Recommended yes, same gate as all other commands.

---

# Part 3 — Side‑by‑side mapping (Go → Rust)

| `gh` component (Go) | File in `cli/cli` | Proposed Flox equivalent (Rust) | File in `flox/flox` |
|---|---|---|---|
| `NewCmdExtension` (root `gh extension` cobra cmd) | `pkg/cmd/extension/command.go` | `extension` bpaf enum + handler dispatch | `cli/flox/src/commands/extension/mod.rs` |
| `extensions.ExtensionManager` interface | `pkg/extensions/extension.go` | `ExtensionManager` trait (async) | `cli/flox-rust-sdk/src/providers/extensions/manager.rs` |
| Concrete `Manager` struct (`dataDir`, `client`, `gitClient`, `platform`, `io`) | `pkg/cmd/extension/manager.go` | `LocalExtensionManager` struct holding `data_dir: PathBuf`, `http: reqwest::Client`, `git: GitCli`, `platform: Platform`, `source: Arc<dyn ExtensionSource>` | `cli/flox-rust-sdk/src/providers/extensions/manager.rs` |
| `extensions.Extension` interface (`Name`/`Owner`/`URL`/`CurrentVersion`/`IsPinned`/`IsBinary`) | `pkg/extensions/extension.go` | `Extension` struct + `ExtensionKind` enum | `cli/flox-rust-sdk/src/providers/extensions/extension.rs` |
| `ExtTemplateType` (Git/GoBin/OtherBin) for `create` | `pkg/extensions/extension.go` | *Deferred to v2* (`CreateTemplate` enum) | `cli/flox-rust-sdk/src/providers/extensions/create.rs` (v2) |
| `Install(repo, pin)` | `manager.go` `installGit` / `installBin` | `install()` method dispatching on `ExtensionKind` | `providers/extensions/manager.rs` |
| `installBin` release‑asset resolver | `manager.go` (the `asset == nil` check referenced in `cli/cli#9608`) | `github::resolve_asset` function | `providers/extensions/github.rs` |
| `Dispatch(args, in, out, err)` | `manager.go` | `dispatch::spawn_extension` free function | `providers/extensions/dispatch.rs` |
| Unknown‑subcommand fallback | `pkg/cmd/root/root.go` | two‑phase bpaf parse in `main.rs` | `cli/flox/src/main.rs` |
| Extensions dir `$XDG_DATA_HOME/gh/extensions/` | `manager.go` (`dataDir()`) | `$XDG_DATA_HOME/flox/extensions/` | `providers/extensions/layout.rs` |
| `manifest.yml` (owner/name/host/tag/ispinned/path) | `manager.go` writes, `extension.go` reads | `state.toml` (`InstalledState`) | `providers/extensions/manifest.rs` |
| `alreadyInstalledError` / `releaseNotFoundErr` / `commitNotFoundErr` / `repositoryNotFoundErr` / `ErrExtensionExecutableNotFound` | `command.go` & `manager.go` | `ExtensionError` variants | `providers/extensions/manager.rs` |
| `checkValidExtension` | `command.go` | `validate_new_install` helper | `providers/extensions/manager.rs` |
| `normalizeExtensionSelector` | `command.go` | `normalize_name` helper (strips `flox-`, `owner/`) | `providers/extensions/manager.rs` |
| `gh-extension-precompile` GitHub Action | separate repo `cli/gh-extension-precompile` | `flox/flox-extension-precompile` (v2 deliverable, out of CLI scope) | n/a |
| `browse` TUI (`pkg/cmd/extension/browse/`) | that package | v3 deliverable | `providers/extensions/browse.rs` (v3) |
| `factory.Factory.ExtensionManager` field (DI) | `pkg/cmdutil/factory.go` | Field on the existing `Flox` shared context struct | wherever `Flox` lives in `flox-rust-sdk` |
| `DisableFlagParsing: true` on `exec` subcmd | `command.go` | `bpaf::any::<OsString, _, _>("ARGS").many()` on the `exec` subcommand (v3) | `commands/extension/exec.rs` (v3) |

The 5 architectural primitives we preserve 1:1:

1. **The `ExtensionManager` as the single orchestrator** — same methods, same name.
2. **Per‑extension directory containing the executable + managed metadata file** — same storage shape, different file name/format (`state.toml` vs. `manifest.yml`).
3. **Three kinds (script / binary / git) detected in the same order** — try release first, fall back to clone.
4. **Dispatch by PATH search under the managed dir with fall‑through to `$PATH`** — same behavior.
5. **Asset‑name substring + explicit override** — the `installBin` algorithm is reproduced literally, modulo the author‑provided platform map.

The one architectural addition — `environment` stanza in the author manifest plus a three‑way `ActivationMode` in dispatch — is the **only** place Flox deliberately diverges from `gh`.

---

# Part 4 — Phased implementation plan (v1)

Milestones are ordered by dependency; sizing is engineer‑weeks (S = ≤1, M = 2–3, L = 4+). Each milestone is independently shippable behind a `FLOX_EXTENSIONS_ENABLE` feature flag until the full set ships.

### M0 — Skeleton and dispatch (S, foundational)

- New module tree under `cli/flox-rust-sdk/src/providers/extensions/`.
- Add `Extension` and `Commands::Extension(Subcmd)` variants; subcommands are stubs that print `unimplemented`.
- Wire the two‑phase `bpaf` parse in `main.rs`.
- `find()` walks `$XDG_DATA_HOME/flox/extensions/` looking for `flox-<name>` and does not yet read `state.toml`.
- Integration test: a pre‑placed `flox-hello` script dispatches correctly and inherits `$FLOX_ENV`.

*Exit criterion:* `flox hello` runs a manually installed `flox-hello` script; `flox extension --help` lists the four subcommands.

### M1 — Manifest + layout + local install (M)

- `AuthorManifest` / `InstalledState` types with `serde`.
- `layout.rs` with XDG‑aware dir resolution and the `.lock` file.
- `flox extension install .` and `flox extension remove` end‑to‑end.
- `flox extension list` reads `state.toml` from every subdir.
- Bats test: init a fake extension dir, install, list, remove.

*Exit criterion:* local developer loop works. Extension authors can iterate without GitHub.

### M2 — GitHub source: git/script install (M)

- `GitHubSource::clone_repo` via shell‑out to `git`.
- Install flow for script/git kinds including `--pin` resolution.
- `upgrade <name>` for git/script (`git fetch`+reset).
- Name‑conflict validation against the compile‑time core‑command list.

*Exit criterion:* `flox extension install flox-examples/flox-hello-script` works end‑to‑end.

### M3 — GitHub source: binary release install (M)

- `resolve_latest` / `list_release_assets` / `download_asset` against GitHub REST.
- Asset resolution: author map → template → substring fallback → darwin‑arm64→darwin‑amd64 fallback.
- Archive extraction (tar/zip) + `chmod +x` + sha256 record.
- `upgrade <name>` for binary.

*Exit criterion:* install a real published `flox-extension` that ships a Linux and macOS binary; confirm upgrade picks up a new tag.

### M4 — Upgrade‑all and polish (S)

- `flox extension upgrade --all [--dry-run] [--force]`.
- Unified table output for `list` and `upgrade` results.
- Good error messages per §2.9.
- Concurrent‑install safety under the `.lock`.

*Exit criterion:* `flox extension upgrade --all --dry-run` produces the expected report in a repo with multiple kinds.

### M5 — Environment integration (M, the Flox‑unique piece)

- `resolve_mode` implementation reading `$FLOX_ENV` and the manifest.
- `Inherit` and `None` modes wired to `tokio::process::Command` with env scrubbing.
- `Pinned` mode delegating to `cli/flox-activations/` for `flox activate -r <ref> -- <cmd>` semantics.
- Bats tests covering all three modes, including the pinned‑but‑already‑in‑that‑env no‑op path.

*Exit criterion:* the user stories in §1.5 (#4, #5, #6) all pass end‑to‑end.

### M6 — Docs, telemetry, and GA checklist (S)

- `flox.dev/docs/extensions/` pages: user guide and author guide.
- Example `flox-examples/flox-hello-*` repos (script, binary, local) added to the `flox-examples` org.
- Telemetry events for install/upgrade/remove/dispatch behind the existing metrics flag.
- Remove the `FLOX_EXTENSIONS_ENABLE` feature flag.

*Exit criterion:* v1 GA'd in a release; `gh`‑style parity confirmed by running the equivalent `gh` smoke tests translated to `flox`.

### Rough total

Approximately **10 engineer‑weeks** for a single engineer on v1, parallelizable to ~4 calendar weeks with two engineers (M0 → M1 → {M2 ∥ M3} → M4 → M5 → M6). M5 is the only milestone that requires deep familiarity with `flox-activations`; everything else is self‑contained.

### Deliberately deferred (not blocking v1 GA)

- `search`, `browse`, `create`, `exec` subcommands.
- FloxHub `ExtensionSource`.
- Checksum/attestation verification.
- A `flox-extension-precompile` GitHub Action (parallel workstream; not needed for v1 GA since authors can hand‑build).
- Nested extension commands (`flox env my-extension`).

---

### Appendix A — Verification notes for reviewers

Several claims in this document are derived from third‑party mirrors (Fossies, PR comments, community blog posts) rather than direct reads of `main` on `cli/cli` and `flox/flox`. Before coding begins, the implementing engineer should verify by opening the following files and confirming the shapes referenced:

- `cli/cli`: `pkg/cmd/extension/command.go`, `pkg/cmd/extension/manager.go` (in particular the `installBin` function; the `if asset == nil` diagnostic is referenced in `cli/cli#9608`), `pkg/extensions/extension.go` (the `ExtensionManager` and `Extension` interfaces).
- `flox/flox`: `cli/flox/src/commands/mod.rs` (to confirm `bpaf` derive usage and the exact shape of the top‑level enum), `cli/flox/src/main.rs`, `cli/flox-rust-sdk/src/providers/` (to confirm the provider‑module pattern), and `cli/flox-activations/` (to understand the public API exposed for the in‑process `activate -- cmd` flow).

Any discrepancy with this document should be resolved in favor of what's in the repos; the design decisions here do not depend on any particular line number, only on the presence of the named concepts (workspace crates, `bpaf` subcommands, `$FLOX_ENV` activation vars, `exec()`‑based child launch in v1.9+).

### Appendix B — Example flows the design must support

**B1. First‑time install and run, no active environment:**

```
$ flox extension install flox-examples/flox-hello
✓ Downloaded flox-hello v1.0.0 (linux-x86_64, 1.2 MB, sha256=fe14…)
✓ Installed to ~/.local/share/flox/extensions/flox-hello

$ flox hello world
Hello, world! (FLOX_ENV is unset)
```

**B2. Install inside an activation:**

```
$ flox activate
flox [myproj] $ flox extension install flox-examples/flox-greet
flox [myproj] $ flox greet
Greetings from myproj! My PATH includes /nix/store/…-myproj/bin
```

**B3. Pinned‑env extension overriding the user's active env:**

```
$ flox activate -d ./dev
flox [dev] $ flox deploy staging
→ activating acme/prod-tools (pinned by flox-deploy manifest)
✓ deploy complete
flox [dev] $               # back in dev
```

**B4. Pinned install:**

```
$ flox extension install --pin v2.0.0-rc.1 flox-examples/flox-tool
$ flox extension list
NAME   REPO                          VERSION        PINNED
tool   flox-examples/flox-tool       v2.0.0-rc.1    yes

$ flox extension upgrade --all
skipping 'tool' (pinned to v2.0.0-rc.1); pass --force to override
```

End of document.