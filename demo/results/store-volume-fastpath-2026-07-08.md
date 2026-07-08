# Store-Volume Fast Path — Results

**Date:** 2026-07-08
**Host:** macOS arm64, Apple Container 1.1.0
**Branch:** sl-002-store-volume-fastpath (prototype/sandboxed-activation)

## Background

Full OCI image bake (~2-5 min) is required today every time the environment
changes, even though the cross-compiled Linux closure already persists in the
`flox-nix` named cache volume. The runtime container only needs the closure
plus an activation context — the image assembly (skopeo conversion, archive
stream, `container image load`) adds no value at run time.

The goal: skip image assembly on activation by mounting the `flox-nix` volume
read-only at `/nix` inside the runtime container, and constructing the
`activateCtx` JSON on the host.

---

## Empirical Gate Results (Requirement 3)

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

**FAIL.** Apple Container 1.1.0 does not support file bind-mounts. Only
directory bind-mounts are supported. **Workaround:** write the activateCtx
JSON into a temp directory and bind-mount the directory instead.

---

## Implementation

The fast path is behind `FLOX_SANDBOX_OCI_STORE_VOLUME=1` (default off).

### How it works

On activation with the valve set:

1. **Valve check**: If `FLOX_SANDBOX_OCI_IMAGE` or
   `FLOX_SANDBOX_OCI_ALLOW_STALE` are set, the fast path is bypassed and the
   default image-based path runs instead.

2. **Env path resolution**: Read the existing image's entrypoint config
   (`container image inspect <env>:latest`) to extract the environment store
   path (the first element of `Entrypoint` minus `/libexec/flox-activations`).
   The `flox-nix` volume must exist.

3. **ActivateCtx JSON**: Write an `activateCtx` JSON file to a temp directory
   on the host with the extracted env store path and the nixos/nix image's
   bash path (`/root/.nix-profile/bin/bash`).

4. **Container run**: Run `nixos/nix:<NIX_VERSION>` with:
   - `--mount type=volume,source=flox-nix,target=/nix,readonly` — closure
     served from the builder volume, no image rebuild needed.
   - `--volume <project>:<project>` — live project mount (same as default path).
   - `--mount type=bind,source=<tmpdir>,target=/run/flox-ctx,readonly` —
     temp directory with the activateCtx JSON inside.
   - `--entrypoint <env>/libexec/flox-activations` — override with the env's
     own `flox-activations` binary (resolves from the mounted volume).
   - `activate --activate-data /run/flox-ctx/activate-ctx.json` — standard
     activation command.

### Staleness handling

The fast path warns (but does not fail) when the expected hash-tag image for
the current lockfile is absent:

```
⚠️  Store-volume fast path: env may have changed since last bake
   (expected image 'sandbox-demo:a7f880489710' not found).
   Running previous closure; re-bake to pick up changes.
```

This allows the fast path to work after a minor env change that does not yet
have a fresh bake. The user sees the warning and knows a fresh bake is needed
to pick up changes.

`FLOX_SANDBOX_OCI_IMAGE` and `FLOX_SANDBOX_OCI_ALLOW_STALE` bypass the fast
path entirely and run the default image-based path.

### Isolation

The store volume is mounted **read-only**. The only writable mounts are:
- The project directory (same as the default path).
- `/tmp` (container tmpfs, ephemeral).

All volume GC and write operations remain host/builder-side.

---

## Timing (warm volume, warm nixos/nix image)

Environment: `sandbox-demo` (bash, coreutils, curl, git) on macOS arm64.
Measured as time from process start to `uname -sm` output.

| Path | Run 1 | Run 2 | Run 3 | Notes |
|------|-------|-------|-------|-------|
| Default (stale image, ALLOW_STALE) | 755 ms | 737 ms | 717 ms | Image already local |
| Fast path (STORE_VOLUME=1) | 793 ms | 867 ms | 846 ms | Volume already populated |

The fast path is **~5-15% slower** than the default stale-image path in the
warm case. This is expected: the fast path adds two extra steps (volume inspect
check, image inspect for env path extraction) and mounts an additional
directory.

### Real-world benefit: after env change

| Scenario | Default path | Fast path |
|----------|-------------|-----------|
| Fresh `flox install <pkg>` | ~2-5 min (full bake) | ~800 ms (reuses old closure, warns) |
| Env in sync with volume | ~800 ms | ~840 ms |

The fast path saves the entire bake time after a minor env change, at the cost
of running the old closure (with a visible warning). A re-bake is still needed
to pick up the new packages — but the user is not blocked from running in the
meantime.

---

## Open Issues

### 1. Cold-start (no prior bake) is unsupported

When no image exists for the environment, `oci_store_volume_env_path_from_image`
returns `None` and the fast path fails. The user must run a full bake first.

A future implementation could call `populate_and_build_env` (the
`ContainerizeProxy` method retained in the codebase) to run a slim builder
step that populates the volume and outputs the env store path via `nix build`,
skipping the image assembly entirely. This would make the first activation
also fast — but requires the builder flake to support it.

### 2. Env changed: stale closure warning requires action

When the lockfile changes after the last bake, the fast path runs the OLD
closure with a warning. This is intentional for the prototype: the user sees
the warning and can re-bake. A stricter mode could fail fast when the hash
doesn't match, requiring an explicit opt-in to run stale.

### 3. nixos/nix base image vs thin busybox

The fast path uses `nixos/nix:<NIX_VERSION>` as the runtime base image. This
is the same image used for builder steps and is already in the local store
after a full bake. It is ~200 MB unpacked, which is large for a runtime
container. A thinner base image (busybox + coreutils) would suffice if
`/root/.nix-profile/bin/bash` is replaced with a bash path from the Nix store.
This is left as future work — the nixos/nix image starts fast once cached.

### 4. Apple Container file bind-mount limitation

Apple Container 1.1.0 does not support file bind-mounts. The activateCtx JSON
is therefore written into a temp directory and the directory is bind-mounted.
This is a minor complexity but not a blocker.

---

## Files Changed

- `cli/flox/src/commands/activate.rs` — adds `FLOX_SANDBOX_OCI_STORE_VOLUME`
  valve constant and the fast-path implementation:
  `oci_store_volume_valve()`, `oci_store_volume_env_path_from_image()`,
  `oci_store_volume_exists()`, `oci_store_volume_env_path()`,
  `oci_store_volume_write_ctx()`, `oci_store_volume_run_argv()`,
  `wrap_activation_oci_store_volume()`. Fast-path check inserted in
  `wrap_activation_oci` before the image-state resolution.

- `cli/flox/src/commands/containerize/macos_containerize_proxy.rs` — adds
  `populate_and_build_env()` method on `ContainerizeProxy` (dead code warning
  suppressed; retained for future cold-start fast path).

- `demo/SCRIPT.md` — documents the new valve in §3.

- `demo/results/store-volume-fastpath-2026-07-08.md` — this file.
