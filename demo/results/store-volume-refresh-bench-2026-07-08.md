# Store-Volume Refresh — Benchmark Results

**Date:** 2026-07-08
**Host:** macOS 26.5.1 arm64, Apple Container 1.1.0
**Branch/rev:** store-volume-refresh @ 847e453e13 (pushed to origin)
**Env:** curl + git + jq closure, one small package added per change.

## Headline

| Path | env-change → activate | vs baseline |
|------|----------------------|-------------|
| Baseline (full OCI rebuild) | **~76 s** | — |
| Store-volume refresh | **~33 s** (32.8–34.6 s) | **2.3× faster** |

The refresh works end-to-end: the guest activates (`uname -sm` →
`Linux aarch64`), the freshness marker is written, and **no image
assembly runs** (zero `streamLayeredImage` layer-creation, no skopeo,
no `container image load`). Verified across three consecutive refreshes.

## Where the 33 s goes (warm refresh, `-vv` split)

| Phase | Time | Note |
|-------|------|------|
| `populate_cache_volume` (`nix copy`, "copying 0 paths") | ~0.7 s | incremental, negligible |
| env closure build (`buildenv.nix`, substituted) | ~0.04 s | negligible |
| **activation-context build (`mkContainer.nix passthru.activateCtx`)** | **~24.4 s** | **the bottleneck** |
| store-volume activation run + 2× VM boot + overhead | ~7–8 s | |

The direct-exec binary cache worked as designed — the flox-flake unpack
(~21 s in a normal bake) is gone; warm refreshes exec the cached
`flox-bin-<rev>` from the volume directly. Refresh #1, #2, #3 are all
~33 s (no repeat speedup), confirming the cost is per-refresh work, not
one-time.

## The remaining bottleneck: nixpkgs evaluation in the context build

`mkContainer.nix` builds `activateCtx` with
`builtins.getFlake nixpkgsFlakeRef` and
`nixpkgs.legacyPackages.aarch64-linux` to resolve `containerPkgs.bash`
(the shell path baked into the activation context). Evaluating — and
re-fetching — nixpkgs costs ~24 s, and it is **redone on every refresh**
even though the nixpkgs-derived part (bash) is constant across env
changes; only the env store path in the context changes.

This is a *second* eval cost, distinct from the flox-flake unpack the
direct-exec cache already eliminated. It was not anticipated in the
~7 s target.

## Path to the ~7 s floor (follow-up)

Cache the nixpkgs-derived constant so the context build skips the
nixpkgs eval:

- Resolve `containerPkgs.bash` (and any other nixpkgs-derived context
  fields) **once**, cache the store path per builder pin, and pass it
  into a lighter context builder that assembles the JSON without
  `getFlake`. The env-specific fields (env path, state dir) are cheap
  and carry no nixpkgs dependency.
- Keep the context byte-identical to the baked one (the design's
  no-drift requirement): derive the cached value from the *same*
  `mkContainer.nix` expression, do not hand-reconstruct it host-side.

Estimated result: ~33 s − ~24 s ≈ **~8–9 s**, approaching the target.
This is a bounded, well-scoped optimization, but it changes
`mkContainer.nix`'s interface and needs its own rev + re-benchmark.

## Reproduce

`demo/results/bench/baseline-bench.sh` (valve off, full rebuild) and
`demo/results/bench/after-bench.sh` (valve on; set `FLOX_BIN` to a
built flox from this branch and `FLOX_REV` to the pushed rev). Run
**unsandboxed** — the refresh spawns `container run`, which a command
sandbox blocks. The first refresh after a new rev builds flox-for-linux
in the VM (one-time) and writes `~/.cache/flox/store-volume/flox-bin-<rev>`.
