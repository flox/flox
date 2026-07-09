# Store-Volume Refresh — Design Spec (skip-rebake fast path)

**Goal:** make `env change → flox activate --sandbox …oci` skip full OCI
image assembly, serving the environment from the shared `flox-nix` store
volume. Measured baseline: a rebuild is **~76s**, of which **~53s is
skippable image assembly** (mkContainer streamLayeredImage build ~21.6s +
layer packing ~23.1s + skopeo/load ~8s). The env-closure build itself is
~1.3s (substituted). Target: **~7s warm**, **~28s** on the first refresh
after a flox-version change (see Binary Resolution).

This closes Open Issue 1 ("the skip-rebake win needs a lighter refresh
step") in `store-volume-fastpath-2026-07-08.md`. Behind the existing
`FLOX_SANDBOX_OCI_STORE_VOLUME=1` valve (macOS/Apple Container only).

## Prior art (deleted, for reference only)

`ContainerizeProxy::populate_and_build_env` (git `44d2093f7`, deleted in
`b807b4310`) had the right *shape* (run a builder container, build into
the volume, print a store path) but the wrong target: it ran `flox build`
then `nix eval '<flake>#packages.aarch64-linux.default.outPath'` — the
flox CLI package, not the environment — and never built the activation
context. Do not reuse its build target; reuse only its argv/mount
plumbing pattern.

## What the refresh must produce

The store-volume run (`oci_store_volume_run_argv`, `activate.rs`) needs an
`OciBakedEntrypoint { env_store_path, activate_ctx_path }`. Today those
come from `container image inspect` of a freshly baked image. The refresh
must produce the same two Linux store paths **without** assembling an
image, and record them host-visibly so activation can skip the bake.

Both paths already exist in the pipeline (from the phase-timing log):
- `env_store_path`: the `environment-run`/`environment-dev` bundle built
  by `BuildEnv::build` from the lockfile (buildenv.nix). ~1.3s substituted.
- `activate_ctx_path`: the `activations-context` `writeTextFile` built
  *inside* `mkContainer.nix` as the `activateCtxStorePath` binding
  (currently not a standalone output). It is tiny.

## Change set

### 1. `mkContainer/mkContainer.nix` — expose the context as passthru

`activateCtxStorePath` is an internal `let` binding consumed only by the
`Entrypoint`. Expose it (and the resolved `environment`) via `passthru`
on the `streamLayeredImage` result so it can be built **without** building
or running the image script:

```nix
passthru = {
  activateCtx = activateCtxStorePath;   # NEW: the activations-context store path
  environment = environment;            # NEW: the resolved env closure
  tests = import ./tests.nix { … };      # unchanged
};
```

`streamLayeredImage` forwards `passthru`. Verify with:
`nix build -f mkContainer.nix passthru.activateCtx --argstr environmentOutPath … --argstr …`
builds only the tiny context derivation (no layer packing). If the
`attrpath`-after-autocall form does not build, fall back to
`nix eval --raw … --apply 'x: x.passthru.activateCtx'` to get the path,
then realise it. Tested with `nix build`, not a Rust unit test (Nix
expression exception in tdd-discipline).

Do **not** change the top-level return shape — `MkContainerNix` still
expects the function result to be the stream-script derivation with
`outputs.out`.

### 2. `flox-rust-sdk` `MkContainerNix` — build just the context

Add a method beside `create_container_source` that reuses the *identical*
argstr wiring (nixpkgsFlakeRef, system, containerSystem, environmentOutPath,
activationMode, interpreterPath, containerName, containerConfigJSON — see
`container_builder.rs:145-177`) but builds `passthru.activateCtx` instead
of the image script, returning the realised store path. Reusing the same
wiring is load-bearing: it guarantees the context is byte-identical to the
baked one (no drift — the whole point of Open Issue 1's warning).

### 3. `flox` `containerize` command — a refresh mode

Add a hidden/experimental flag to the inner command
(`cli/flox/src/commands/containerize/mod.rs`), e.g.
`--store-volume-refresh`, that:
1. Builds the environment (existing `BuildEnv::build`) → run bundle path.
2. Builds the activation context (change 2) → ctx path.
3. Resolves its own binary path: `readlink /proc/self/exe` (the Linux
   flox store path inside the builder).
4. Prints one line of JSON to stdout and assembles **no** image:
   `{"env_run":"/nix/store/…","activate_ctx":"/nix/store/…","flox_bin":"/nix/store/…-flox-…/bin/flox"}`

This runs inside the builder VM (Linux), same context as the normal inner
`flox containerize`. It reuses all existing env/interpreter/mode wiring.

### 4. `macos_containerize_proxy.rs` — the host refresh method + binary cache

Add `refresh_store_volume(&self, flox) -> Result<StoreVolumeRefresh, …>`
returning `{ env_run, activate_ctx, flox_bin }`:

1. `populate_cache_volume()` (existing — incremental `nix copy`).
2. Resolve the invocation:
   - **Binary Resolution.** Read a host cache file
     `~/.cache/flox/store-volume/flox-bin-<builder_pin>` (builder pin =
     `FLOX_VERSION.commit_sha()` / the `_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV`
     rev). If present and the path exists in the volume, exec it
     **directly**: `container run … <flox_bin> containerize
     --store-volume-refresh --dir /flox_env` — **no `nix run`, no flake
     unpack (~7s path).**
   - On cache miss, fall back to the flake:
     `container run … bash -c "nix … run 'github:flox/flox/<rev>' --
     containerize --store-volume-refresh --dir /flox_env"` (~28s: pays the
     flake unpack + a one-time flox-linux build for a new rev). Parse the
     `flox_bin` from the JSON and **write the cache file** so the next
     refresh is fast.
3. Parse the JSON from stdout; validate all three are `/nix/store/…`.

Mounts identical to the existing builder run (`add_runtime_args`): env
bind at `/flox_env`, `flox-nix` volume at `/nix`, flox.toml, NIX_CONFIG.

### 5. `activate.rs` `wrap_activation_oci` — short-circuit the bake

This is the behavioral core. When the store-volume valve is on and the
platform is macOS, **before** the image-state resolution / bake flow:

1. Compute the current environment's lockfile hash12 (same value used for
   the image tag today — reuse that code).
2. Read the refresh marker `<.flox/cache>/store-volume-refresh.json`
   = `{ lockfile_hash12, env_run, activate_ctx, builder_pin }`.
3. **Fresh hit:** marker.lockfile_hash12 == current AND both paths exist
   in the volume → build `OciBakedEntrypoint { env_run, activate_ctx }`
   and dispatch `oci_store_volume_run_argv` directly. No bake. (~<1s.)
4. **Miss/stale:** run `refresh_store_volume` (change 4), write the marker,
   then dispatch the store-volume run. (~7s warm / ~28s cold.)
5. **Refresh failure:** fall back to the existing bake + store-volume path
   (never a silent stale run — preserve current safety).

`FLOX_SANDBOX_OCI_IMAGE` and `FLOX_SANDBOX_OCI_ALLOW_STALE` still bypass
the valve entirely (unchanged). The valve remains off by default.

The staleness guarantee is preserved: the marker's `lockfile_hash12` is
the freshness proof, exactly as the image tag was. A changed lockfile ⇒
marker miss ⇒ refresh (which rebuilds env+ctx for the new lock). A stale
marker can never run because the hash won't match.

## Unit tests (TDD where it's pure logic)

- `store_volume_refresh.json` (de)serialization + hash-match predicate
  (fresh/stale/missing → run-directly / refresh / refresh).
- JSON parse of the inner command's stdout line (valid + malformed +
  non-store-path rejection), mirroring `parse_baked_entrypoint`'s
  rejection-shape tests.
- Binary-cache file read/write + builder-pin keying (hit → direct-exec
  argv; miss → nix-run argv), asserted via exact-argv snapshots like the
  existing `oci_store_volume_run_argv` tests.
- The refresh argv builders are pure functions of their inputs (pass
  `flox_bin`, paths, invocation in) so argv is snapshot-testable without a
  container.

Nix change (mkContainer passthru) is verified with `nix build`, not a
Rust test.

## Out of scope / handled by the human (not IW)

- Pushing this branch to the flox origin (the builder fetches the rev).
- End-to-end bakes + the before/after benchmark.
- Volume GC of refresh paths (Open Issue 2 — unchanged).

## Verification the human will run (end-to-end)

With the branch pushed and `_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV=<rev>`:
create env → cold bake (warms the volume + builds flox-linux) → env change
→ `FLOX_SANDBOX_OCI_STORE_VOLUME=1 flox activate …` and confirm (a) it
skips image assembly, (b) the guest activates and `uname` works, (c) wall
time vs the 76s baseline. Then a second env change to measure the warm
(~7s, cached binary) path.

---

## Addendum: context-build optimization (33s → ~8-9s)

Benchmark found the warm refresh is ~33s, of which **~24s is
`create_activate_ctx`** — `mkContainer.nix` runs `builtins.getFlake
nixpkgs` + `nixpkgs.legacyPackages.aarch64-linux` to resolve the baked
`bash` path, redone on every refresh though the bash path never changes
across env changes. A nixpkgs-free `builtins.toFile` context build
evaluates in ~0.1s (verified). Only `shell.bash` is nixpkgs-derived; all
other context fields are passed in.

### Change set (no drift — one source of truth for the context attrs)

1. **`mkContainer/activate-ctx.nix`** (new, pure, no nixpkgs): a function
   `{ bashPath, environmentOutPath, interpreterPath, activationMode,
   containerName }:` returning the **exact** `activateCtx` attrset now
   inlined in `mkContainer.nix` (lines ~102-130), with only the sourced
   values parameterized: `shell.bash = bashPath` (was
   `"${containerPkgs.bash}/bin/bash"`), `flox_activate_store_path`/
   `attach_ctx.env` = `environmentOutPath` (was `environment`),
   `activation_state_dir = ".../${baseNameOf environmentOutPath}"`,
   `interpreter_path = interpreterPath`, `*_description`/`*_environments
   = containerName`. Uses only builtins (`baseNameOf`). Returns the
   attrset (not JSON, not a store path).

2. **`mkContainer.nix`**: replace the inline `activateCtx = { … }` with
   `activateCtx = import ./activate-ctx.nix { bashPath =
   "${containerPkgs.bash}/bin/bash"; environmentOutPath = "${environment}";
   interpreterPath = "${interpreterPath}"; inherit activationMode
   containerName; };`. Keep `activateCtxJson`/`activateCtxStorePath`
   (writeTextFile) and `passthru.activateCtx` unchanged. **Verify the
   emitted context JSON is byte-identical to the pre-refactor output**
   (diff the toJSON) — existing baked images must not change behavior.

3. **`mkContainer/refresh-activate-ctx.nix`** (new, no nixpkgs): same args
   as (1); returns `builtins.toFile "activations-context" (builtins.toJSON
   (import ./activate-ctx.nix { … }))`. Args arrive as `--argstr` (plain
   strings, no string context) so `toFile` is allowed. Byte-identical JSON
   to (2) for the same inputs (same `activate-ctx.nix`, same values).

4. **`MkContainerNix::create_activate_ctx_fast(flox, bash_path)`**: builds
   `refresh-activate-ctx.nix` with the same argstrs as `create_activate_ctx`
   plus `--argstr bashPath <bash_path>` (no `containerConfigJSON`,
   `nixpkgsFlakeRef`, `system`, `containerSystem` — none are needed).
   `builtins.toFile` realises during eval, so `nix eval --raw --file … `
   returns the path; no nixpkgs, ~0.1s.

5. **Inner `--store-volume-refresh` (`mod.rs`)**: accept optional
   `--container-bash <path>`. If present → `create_activate_ctx_fast`;
   set `container_bash` = the arg. If absent → `create_activate_ctx`
   (slow, nixpkgs) and read `shell.bash` from the built context JSON to
   report `container_bash`. Emit it in the JSON line.

6. **`StoreVolumeRefresh`**: add `container_bash` (validate
   `/nix/store/…/bin/bash`). Update parse/to_json_line/is_valid + tests.

7. **Host proxy `refresh_store_volume`**: add a `container-bash-<pin>`
   cache beside `flox-bin-<pin>`. On hit, pass `--container-bash <path>`
   to the inner invocation (both direct-exec and nix-run argv). After a
   refresh, persist the returned `container_bash`.

Amortization mirrors the flox-bin cache: first refresh after a
flox-version (nixpkgs-pin) change pays the ~24s nixpkgs eval once and
caches bash; subsequent refreshes build the context nixpkgs-free (~0.1s),
landing the warm refresh at ~8-9s.
