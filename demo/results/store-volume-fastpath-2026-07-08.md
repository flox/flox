# Store-Volume Run — Results

**Date:** 2026-07-08 (revised same day after code review)
**Host:** macOS arm64, Apple Container 1.1.0
**Branch:** sl-002-store-volume-fastpath (prototype/sandboxed-activation)

## Background

Full OCI image bake (~2-5 min) is required today every time the environment
changes, even though the cross-compiled Linux closure already persists in the
`flox-nix` named cache volume. The runtime container only needs the closure
plus an activation context — the image assembly (layer packing, skopeo
conversion, archive stream, `container image load`) adds no value at run time.

The exploration goal: skip image assembly on activation by mounting the
`flox-nix` volume read-only at `/nix` inside the runtime container.

## Outcome summary (honest version)

The mechanics work: a guest can run the environment entirely from the
volume, read-only, with the baked entrypoint and activation context
resolving from the mounted store. All empirical gates pass.

The prototype does **not** yet deliver the skip-rebake win. Freshness is
only provable via the hash-tagged image (lockfile-hash → image tag is the
only host-visible freshness marker), so an env change still goes through
the normal bake flow before the store-volume run can proceed. Closing that
gap needs a lighter builder-side "refresh" step that builds the environment
derivation and records the (lockfile-hash → env path, ctx path) mapping
without assembling an image — see Open Issues.

What the prototype does deliver:

- Verified run-from-volume mechanics (gates below) and a working,
  staleness-safe implementation behind `FLOX_SANDBOX_OCI_STORE_VOLUME=1`.
- Machine-checkable mount-surface tests (exact argv per invocation type).
- A precise map of what Apple Container 1.1.0 supports and where the
  remaining engineering is.

---

## Empirical Gate Results

All gates tested against Apple Container 1.1.0.

### Gate 1: Named volume mount at run time

```
$ container run --rm \
    --volume flox-nix:/test-nix-store \
    busybox:latest ls /test-nix-store/store | head -5
00067rghcrqknxg0cdgb5zgfsb6avscs-lame-3.100.tar.gz.drv
00d0r43qsbck8rmpvvjbygrqyifc3szf-cabal-doctest-1.0.12.tar.gz.drv
...
Exit code: 0
```

**PASS.** Named volumes can be mounted at container run time.

### Gate 2: Read-only volume mount

```
$ container run --rm \
    --mount "type=volume,source=flox-nix,target=/nix,readonly" \
    busybox:latest ls /nix/store | head -3
00067rghcrqknxg0...
...
Exit code: 0
```

**PASS.** Read-only mounts are supported via `--mount type=volume,...,readonly`.

### Gate 2b: Writes blocked in read-only mount

```
$ container run --rm \
    --mount "type=volume,source=flox-nix,target=/nix,readonly" \
    busybox:latest sh -c 'touch /nix/test-write 2>&1; echo "exit: $?"'
touch: /nix/test-write: Read-only file system
exit: 1
```

**PASS.** Writes are blocked at the filesystem level.

### Gate 3: Entrypoint executable from volume

```
$ container run --rm \
    --mount "type=volume,source=flox-nix,target=/nix,readonly" \
    busybox:latest \
    /nix/store/yw9dl3wqj4pxgqak1kqq5q1q88igxnjv-flox-activations-1.13.1/libexec/flox-activations --help
Monitors activation lifecycle...
Exit code: 0
```

**PASS.** Binaries in the volume are executable from the runtime container.

### Gate 4: Concurrent read-write + read-only access

```
$ container run --rm -d --name rw-test \
    --mount "type=volume,source=flox-nix,target=/nix" \
    busybox:latest sleep 30
$ container run --rm \
    --mount "type=volume,source=flox-nix,target=/nix,readonly" \
    busybox:latest ls /nix/store | head -3
...
Exit code: 0
```

**PASS.** A builder container can hold the volume read-write while a runtime
container mounts it read-only concurrently.

### Gate 5: File bind-mount (FAIL — limitation)

```
$ container run --rm \
    --mount "type=bind,source=/tmp/test.json,target=/tmp/test.json,readonly" \
    busybox:latest cat /tmp/test.json
Error: path '/tmp/test.json' is not a directory
```

**FAIL.** Apple Container 1.1.0 does not support file bind-mounts; only
directory bind-mounts work. The current design does not need any file
bind-mount (the activation context is read from the volume), so this is a
recorded limitation rather than a live constraint.

---

## Implementation

The store-volume run is behind `FLOX_SANDBOX_OCI_STORE_VOLUME=1`
(default off, macOS/Apple Container only — setting it on Linux is a hard
error).

### How it works

Image-state resolution and staleness handling are exactly the default
path's: the content-hash tag (`<env>:<hash12>`) is resolved first, and a
missing or stale image goes through the normal bake flow (tty prompt or
`FLOX_SANDBOX_OCI_AUTOBAKE`). Only when a **fresh** closure exists — the
hash-matched image is present, or a bake just completed — does the valve
change the final run step:

1. **Entrypoint extraction**: `container image inspect <env>:<hash12>`
   records the baked entrypoint:

   ```
   ["<env-store-path>/libexec/flox-activations", "activate",
    "--activate-data", "<activations-context-store-path>"]
   ```

   Both store paths were written into the builder's store — which *is* the
   `flox-nix` volume — during the bake, so they resolve from a container
   that mounts the volume at `/nix`.

2. **Container run**: `nixos/nix:<NIX_VERSION>` with exactly two mounts:
   - `--mount type=volume,source=flox-nix,target=/nix,readonly`
   - `--volume <project>:<project>` (live project mount, same as default)

   The entrypoint override reproduces the baked image's own entrypoint
   verbatim (env's `flox-activations` + baked activations-context).

There is **no host-side reconstruction of the activation context**. The
baked context in the volume is the authoritative artifact produced by
mkContainer.nix at bake time; reusing it verbatim means the activation
mode (dev/run), interpreter path, and shell path are exactly as baked —
there is no second copy of the contract to drift. (The baked context's
`shell.bash` points at a `containerPkgs.bash` store path, which resolves
from the volume; the base image's own profile is not involved.)

Falls back to running the baked image (which is self-contained) when the
volume is missing or the entrypoint does not match the contract.

### Staleness semantics

Identical to the default path by construction. The valve is consulted only
after image-state resolution:

| State | Behavior with valve set |
|-------|------------------------|
| Fresh (`<env>:<hash12>` present) | Store-volume run |
| Just baked | Store-volume run (closure was refreshed by the bake) |
| Stale / missing | Normal bake flow first (prompt / AUTOBAKE / fail-fast) |
| `FLOX_SANDBOX_OCI_IMAGE` set | Valve bypassed; explicit image runs as-is |
| `FLOX_SANDBOX_OCI_ALLOW_STALE` | Valve bypassed; stale image runs as-is |

A stale or mismatched closure can never run silently: the env store path
is only ever read from the hash-matched (or just-baked) image.

### Isolation: the read surface is wider than the baked image

The store-volume run trades image assembly for a **larger read-only
surface**. State this clearly when reasoning about the sandbox boundary:

- The guest mounts the **entire `/nix`** from the volume — not just
  `/nix/store`, and not just this environment's closure. That includes:
  - The **union of every closure ever baked** on this machine, across all
    environments — another project's packages are readable.
  - **`.drv` files** (build recipes, which can embed URLs and metadata).
  - `/nix/var` (Nix database, profiles, gcroots) and the **builder's full
    Nix toolchain** — `nix` itself is executable from the volume.
- The guest runs as **root**, same as the baked-image path.
- Everything is **read-only** (Gate 2b): no store path can be written or
  removed from inside the sandbox; volume writes and GC remain
  host/builder-side operations.

By contrast, the default baked image exposes only the single environment's
closure. For threat models where cross-environment package visibility or
`.drv` metadata matters, the store-volume run widens what a compromised
workload can *read* (not write). A future hardening step could mount only
the closure's paths, but Apple Container has no per-path mount filtering,
so that would require a per-env volume or an overlay mechanism.

The mount set is enforced by unit tests that assert the **exact argv** per
invocation type — exactly one read-only volume mount and one project bind
mount, no other channels.

### No flox shim on exec paths

Non-interactive invocations (`flox activate -- cmd` and the `sh -c` form)
have **no `flox` shim in the guest** — there is no rcfile, so no `flox`
command exists at all (verified live). Only interactive sessions get the
minimal shim (`flox deactivate` works; other subcommands print a notice
and return 127), because it is defined by the generated rcfile. This is
the same behavior as the baked-image path.

---

## Timing (warm volume, warm nixos/nix base image)

Environment: `sandbox-demo`-class env (bash, coreutils, curl, git) on
macOS arm64. Time from process start to `uname -sm` output.

| Path | Run 1 | Run 2 | Run 3 | Notes |
|------|-------|-------|-------|-------|
| Default (baked image) | 755 ms | 737 ms | 717 ms | Image already local |
| Store-volume run | 793 ms | 867 ms | 846 ms | Volume already populated |

The store-volume run is ~5-15% slower warm: it adds a volume-existence
probe and an image inspect before the run. After an env change, **both
paths go through the same bake flow** (~2-5 min) — the valve does not skip
it (see Outcome summary and Open Issues).

---

## Open Issues

### 1. The skip-rebake win needs a lighter refresh step (the crux)

Freshness is currently only provable via the hash-tagged image, so a
changed env must complete a full bake before the store-volume run can
proceed. To actually skip image assembly on env change, a builder step
would need to:

1. Run `nix copy` into the volume (incremental — already exists as the
   populate step) and build the environment derivation in the builder
   (`nix build --file buildenv.nix --argstr manifestLock …`, as
   `flox containerize` does internally).
2. Build the activations-context for that env (what mkContainer.nix's
   `activateCtx` does at image-build time).
3. Record the `(lockfile-hash12 → env path, ctx path)` mapping somewhere
   host-visible (e.g. `.flox/cache/`), replacing the image tag as the
   freshness marker.

A first attempt at (1) shipped in an earlier revision of this branch as
`ContainerizeProxy::populate_and_build_env` and was deleted during review:
its primary branch evaluated the flox CLI package's outPath rather than
the built environment, and it hardcoded `aarch64-linux`. The git history
of this branch preserves it as a reference for the argv/mount plumbing
only — the eval target needs to be the buildenv derivation.

### 2. Volume GC could orphan a fresh image's paths

Nothing currently garbage-collects the `flox-nix` volume, but if a future
builder-side GC lands, the store-volume run's paths (env bundle and
activations-context) must be gcroots — otherwise a hash-matched image
could reference paths pruned from the volume. The fallback (run the baked
image) covers the failure, but only after a confusing in-guest error.

### 3. nixos/nix base image is heavyweight for a runtime shell

`nixos/nix:<NIX_VERSION>` (~625 MB unpacked) is used because it is already
local after any bake (it is the builder image). Since the baked activation
context resolves bash from the volume, a much thinner base (even one with
no shell of its own) should work; untested.

### 4. Apple Container file bind-mounts unsupported

Recorded from Gate 5. Not currently load-bearing (no file bind-mounts in
the design), but relevant to any future host-generated-context variant.

---

## Files Changed

- `cli/flox/src/commands/activate.rs` —
  `FLOX_SANDBOX_OCI_STORE_VOLUME` valve constant;
  `parse_store_volume_valve` / `oci_store_volume_valve`;
  `OciBakedEntrypoint` + `parse_baked_entrypoint` (pure) +
  `oci_baked_entrypoint` (inspect); `oci_store_volume_exists`;
  `oci_store_volume_run_argv` (pure argv builder); dispatch in
  `wrap_activation_oci` after image-state resolution, gated on a fresh
  closure. Unit tests: valve parsing table, entrypoint contract parsing
  (valid + 4 rejection shapes), exact-argv snapshots per invocation type.

- `demo/SCRIPT.md` — valve documented in §3 with staleness table, the
  read-surface isolation note, and timing; §0 shim claim scoped to
  interactive sessions.

- `demo/results/store-volume-fastpath-2026-07-08.md` — this file.
