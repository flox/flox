# Workstream F — Risk Map & Review-Labeling Proposal

Date: 2026-06-11
Status: analysis only (proposal — no changes to `.github/CODEOWNERS` or any
production file are made by this document)

## Context

Goal 4 of `GOAL.md` asks for a structure in which "this is tricky, review
carefully" vs. "this is thin glue" is legible from the diff path alone, and
enforceable via CODEOWNERS. This document identifies the high-consequence
regions of the flox codebase, measures bug-fix commit density per region,
compares against the current CODEOWNERS coverage (one line:
`/home/user/flox/.github/CODEOWNERS`), and proposes a risk-tier taxonomy, a
CODEOWNERS scheme, and an assessment of a `// TRICKY:` comment convention.

Two measurement caveats apply throughout (see "Assumptions"):

1. **The local clone is shallow.** `git rev-parse --is-shallow-repository`
   returns `true`; history contains 457 commits, the oldest dated
   2026-03-10. The requested `--since="2025-01-01"` window therefore
   effectively measures the **last ~3 months** of development. Densities are
   real but the sample is short; areas quiet in this window (e.g.
   `ld-floxlib/`, 1 commit) are not necessarily low-risk — they are
   low-*churn*, which combined with high blast radius is its own risk
   profile ("rarely touched, easily broken, hard to re-learn").
2. **Repo-wide baseline:** 115 of 457 commits (25.2%) are `fix:`-prefixed,
   so per-path fix ratios should be read against a ~25% baseline, not
   against zero.

One path correction relative to the GOAL.md/AGENTS.md candidate list: the
activation shell scripts live at `assets/environment-interpreter/` (verified
via `ls /home/user/flox/assets/`), not `assets/activation-scripts/`. No
commit in available history touches an `assets/activation-scripts` path.
AGENTS.md ("Key Directories" table) still names `assets/activation-scripts/`
— a documentation staleness worth fixing when AGENTS.md is next edited.

---

## Risk map

Fix-density evidence is from
`git -C /home/user/flox log --oneline --since="2025-01-01" -- <path>`
with `fix:` counted by `grep -cE "^[0-9a-f]+ fix(\(|:|!)"` (conventional
commits). Format: `fix/total`.

| Path | Tier | Rationale | Evidence |
|---|---|---|---|
| `ld-floxlib/` | **1** | C shared library injected via `LD_AUDIT` into every dynamically linked process in an activated env. Every libc call requires explicit `.symver` GLIBC bindings in two arch blocks (`GLIBC_2.17` aarch64, `GLIBC_2.2.5` x86_64); a missing binding silently breaks portability on older hosts and is invisible in CI on new glibc. | Structural; AGENTS.md has a dedicated caution section ("GLIBC version binding requirement"); bindings visible at `ld-floxlib/ld-floxlib.c:35-77`. Churn 0/1 — dormant, not safe. |
| `assets/environment-interpreter/` | **1** | Bash/zsh/fish/tcsh scripts sourced or eval'd in **every user shell** on activation. Bugs corrupt user prompts, PATH, completion state, or leak vars across nested activations. Untyped, shell-dialect-sensitive, hard to test exhaustively. | 2/10 fixes incl. `120d8b4 fix(activate): unset FLOX_ZSH_INIT_SCRIPT…`, `aabdd49 fix(deactivate): unbreak prompt restore in fish and tcsh`. AGENTS.md cautions: `FLOX_ACTIVATE_TRACE` debugging section; tcsh quoted-backtick eval rule. |
| `cli/flox-activations/` | **1** | Activation/attach/deactivate state machine and process monitoring (`src/attach.rs`, `deactivate.rs`, `env_diff.rs`, `start_diff.rs`). Manages cross-process state (`state.json`), env diffing, and shell rc generation — the layered/nested activation cases are the bug magnet. | **16/55 fixes (29%)** — highest absolute fix count of any crate-level path. Examples: `f01bf01 fix(deactivate): fix layered in-place deactivation`, `a96a7a2`/`16511b0` (zsh compinit ordering across deactivation levels). |
| `cli/flox/src/commands/activate.rs` | **1** | Process `exec` and fd/stdio inheritance: replaces the CLI process (`command.exec()` at line 597, "exec should never return"), manages stdio inheritance (`activate.rs:583`) and nested-activation env leakage (`activate.rs:514`). Mistakes hang shells or orphan services. GOAL.md classifies this as the "structural" interactivity case that can never be a pure API call. | 4/18 fixes; 1,240 LOC (largest command file); structural argument above. |
| Auth/token handling: `cli/flox-catalog/src/auth/`, `cli/flox-catalog/src/token.rs`, `cli/flox-rust-sdk/src/providers/git_auth.rs`, `cli/flox-rust-sdk/src/providers/nix_auth.rs`, `cli/flox/src/commands/auth.rs` | **1** | Credential material: OAuth device flow and token persistence to config (`commands/auth.rs:216-217`, `floxhub_token` in `cli/flox/src/config/mod.rs`); injection of bearer tokens into git subprocesses via inline credential helpers (`git_auth.rs` — per-variant Auth0/Kerberos/no-material behavior). Token leakage into logs, argv, or error messages is a security incident, not a bug. | Structural (security). Low churn (git_auth 0/6, catalog/auth 1/9, nix_auth 1/5) — review intensity here is about consequence, not frequency. AGENTS.md: "Credential sanitization … belong in `Display` impls or `From` conversions". |
| `cli/flox-rust-sdk/src/providers/git.rs` | **1** | All git subprocess execution plus the typed remote-error classification (`GitRemoteCommandError::{AccessDenied, Diverged, RefNotFound}`, `git.rs:690-696, 774-815`) that the rest of the codebase is forbidden from re-deriving by string matching. Misclassification breaks push/pull UX and the no-string-matching architecture rule. | 1/5 fixes; AGENTS.md has a dedicated "Error handling architecture" caution naming this exact hierarchy. |
| `cli/flox-manifest/src/parsed/`, `src/migrate/`, `src/raw/` (schema & migration) | **1** | The type-state manifest lifecycle (`Manifest<S>`) and schema versioning. AGENTS.md devotes its longest convention section to this crate: shape changes require a **new schema version**; hand-serialization is banned; all reads must go through migrating constructors. A wrong migration silently corrupts user manifests at scale. | 4/17 fixes crate-wide, 1/5 in `src/migrate`; AGENTS.md "Manifest usage (`flox-manifest` crate)" section is itself the strongest evidence of known trickiness. |
| `cli/flox-manifest/src/lockfile/` + `cli/schemas/` | **1** | Lockfile parse/serialize (`lockfile/mod.rs:67-85`) and the published JSON schemas (`lockfile-v1.schema.json`, `manifest-v1.schema.json`, `generations-metadata-v2.schema.json`). Lockfiles are a cross-tool, cross-version compatibility contract (floxhub consumes them — the only path currently in CODEOWNERS is `cli/schemas/`, co-owned by `@flox/floxhub`). | 0/5 fixes (stable, contract-like); existing CODEOWNERS entry is prior evidence the org treats schema as high-consequence. |
| `cli/shell_gen/` | **1** | Generates shell code that user shells `eval`. Same blast radius as `assets/environment-interpreter/`; the tcsh quoted-backtick rule in AGENTS.md ("Generating shell code for tcsh") applies to its output. | Structural; AGENTS.md caution. |
| `nix-plugins/` | **1 (borderline 2)** | C++ Nix plugins — native code loaded into the Nix evaluator; crashes take down evaluation. Low churn in window. | 0/1 fixes; structural argument (native, separate Meson build, few people touch it). |
| `cli/flox-rust-sdk/src/providers/publish.rs` | **2 (watch)** | Publish pipeline: highest fix *ratio* of the measured Rust paths — temp-dir lifecycle, netrc creation timing, publisher-mode waits. Not tier 1 because failures are loud and local (a failed publish), not corrupting, but it is the most actively bug-fixed provider. | **10/29 fixes (34%)**: `6fbdf50`, `0fc9693`, `775bf1b` (netrc/tempdir sequencing). |
| `cli/flox-rust-sdk/src/providers/buildenv.rs` | **2** | Environment realization via Nix; bugs are visible build failures. | 6/18 fixes (33%). |
| `cli/flox-core/` | **2** | Shared utilities (activations, paths, versions) used by everything; also the known crossterm layering leak (GOAL.md baseline). Wide fan-in argues for tier 2 rather than 3. | 7/22 fixes (32%). |
| `cli/flox-rust-sdk/src/models/` (incl. `environment/`) | **2** | Core env models (managed/remote/project, floxmeta). Substantial logic, typed, well-tested; normal review. | models 4/25; `models/environment` 3/23. |
| `cli/flox/src/commands/` (excluding `activate.rs`, `auth.rs`) | **2 today → 3 target** | Command layer currently interleaves logic with I/O (GOAL.md: install.rs ~1,090 LOC). Once commands become parse→call→render (Workstreams A/B), the residue is tier-3 glue. | 20/71 fixes (28%) — today's density justifies tier 2 until thinned. |
| `cli/flox/src/utils/` (messages, dialogs), `cli/flox/src/commands/mod.rs` wiring | **3** | Rendering, message formatting, bpaf wiring. Mistakes are cosmetic or caught instantly by usage. | Structural. |
| `test_data/`, `cli/tests/`, `cli/mk_data/`, `cli/flox-test-utils/` | **3** | Test fixtures and harnesses; cannot break users directly. | Structural. |
| Docs (`*.md`), `img/` | **3** | No runtime effect. | Structural. |

---

## Current CODEOWNERS coverage vs. the risk map (gap analysis)

Current `/home/user/flox/.github/CODEOWNERS` is one line:

```
/cli/schemas/ @flox/cli @flox/floxhub
```

| Tier-1 region | Covered today? |
|---|---|
| `cli/schemas/` | **Yes** — the only covered path |
| `ld-floxlib/` | No |
| `assets/environment-interpreter/` | No |
| `cli/flox-activations/` | No |
| `cli/flox/src/commands/activate.rs` | No |
| Auth/token paths | No |
| `cli/flox-rust-sdk/src/providers/git.rs` | No |
| `cli/flox-manifest/` (schema/migrate/lockfile) | No — notable because lockfile/manifest *shape* is exactly what the existing `cli/schemas/` entry tries to protect, yet the Rust source that defines the shape is uncovered |
| `cli/shell_gen/`, `nix-plugins/` | No |

Conclusion: coverage is ~1 of 10 tier-1 regions. Ownership of everything
else is implicit in commit history (matches the GOAL.md baseline finding).
The sharpest single gap is the manifest/lockfile split: changing
`cli/flox-manifest/src/parsed/v*.rs` can invalidate the schema contract
without touching `cli/schemas/`, so today's rule can be bypassed by the
exact change it exists to catch.

---

## Proposal

### (a) Risk-tier taxonomy

- **Tier 1 — mandatory careful review.** Paths where a defect corrupts user
  state, leaks credentials, breaks every shell, or violates a
  compatibility contract; or where correctness depends on invariants not
  expressible in the type system (`.symver` lists, shell quoting, exec/fd
  semantics, schema migration). Mechanics: CODEOWNERS entry with a named
  owning team; review must confirm the relevant invariant explicitly
  (checklist item, not vibes). At least one reviewer from the owning team.
- **Tier 2 — normal review.** Substantive logic with typed interfaces and
  test coverage where failures are loud and recoverable (providers, models,
  fat command bodies). Standard single-approval review. Optional CODEOWNERS
  entry for routing (not gating).
- **Tier 3 — light review.** Rendering, wiring, fixtures, docs. Reviewable
  by anyone; the review question is "does the output look right", not "is
  the logic sound".

Tier assignment uses three signals, any one of which is sufficient for
tier 1: (i) blast radius (every-shell, credential, or on-disk-contract
code), (ii) an encoded caution in AGENTS.md (each caution is a scar from a
past incident), (iii) sustained fix density well above the 25% repo
baseline **combined with** cross-process or stateful behavior (this is what
elevates `flox-activations` but leaves `publish.rs` at tier 2-watch).

### (b) Path → tier mapping

As tabulated in the risk map above. Summary of tier 1:
`ld-floxlib/`, `assets/environment-interpreter/`, `cli/flox-activations/`,
`cli/flox/src/commands/activate.rs`, auth/token paths
(`cli/flox-catalog/src/auth/`, `cli/flox-catalog/src/token.rs`,
`cli/flox-rust-sdk/src/providers/{git_auth,nix_auth}.rs`,
`cli/flox/src/commands/auth.rs`),
`cli/flox-rust-sdk/src/providers/git.rs`, `cli/flox-manifest/`,
`cli/schemas/`, `cli/shell_gen/`, `nix-plugins/`.

Two deliberate judgment calls:

- `publish.rs` stays tier 2 despite the highest fix ratio (34%): its
  failures are immediate and non-corrupting, and its churn reflects active
  feature work (netrc/tempdir sequencing) rather than fragile invariants.
  Re-measure next quarter; promote if density persists after the feature
  settles.
- `ld-floxlib/` and `nix-plugins/` are tier 1 despite near-zero churn:
  review intensity must price in consequence and re-learning cost, not just
  frequency.

### (c) Proposed CODEOWNERS scheme (proposal only — not applied)

Team names beyond the two that exist in the current file (`@flox/cli`,
`@flox/floxhub`) are **placeholders** to be mapped to real GitHub teams.

```
# ---- Tier 1: mandatory careful review (gating) ----------------------
# Native code in every user process / the Nix evaluator
/ld-floxlib/                                  @flox/cli @flox/activation-owners
/nix-plugins/                                 @flox/cli @flox/nix-owners

# Shell code sourced by every user shell, and its generators
/assets/environment-interpreter/              @flox/activation-owners
/cli/shell_gen/                               @flox/activation-owners
/cli/flox-activations/                        @flox/activation-owners
/cli/flox/src/commands/activate.rs            @flox/activation-owners

# Credential material
/cli/flox-catalog/src/auth/                   @flox/security-owners
/cli/flox-catalog/src/token.rs                @flox/security-owners
/cli/flox-rust-sdk/src/providers/git_auth.rs  @flox/security-owners
/cli/flox-rust-sdk/src/providers/nix_auth.rs  @flox/security-owners
/cli/flox/src/commands/auth.rs                @flox/security-owners

# Git subprocess + remote-error classification hierarchy
/cli/flox-rust-sdk/src/providers/git.rs       @flox/cli

# Manifest/lockfile schema contract (source of truth + published schemas)
/cli/flox-manifest/                           @flox/cli @flox/floxhub
/cli/schemas/                                 @flox/cli @flox/floxhub

# ---- Tier 2: routing only (review by any maintainer; entry exists so
#      the owning team is notified, not to gate) ----------------------
/cli/flox-rust-sdk/                           @flox/cli
/cli/flox-core/                               @flox/cli

# Tier 3 paths intentionally have no entry.
```

Notes on mechanics:

- CODEOWNERS is last-match-wins, so tier-1 file-level entries must come
  *after* the tier-2 directory entries if reordered; as written above,
  the `/cli/flox-rust-sdk/` routing line would shadow `git.rs` and
  `git_auth.rs`/`nix_auth.rs` — in the real file, place tier-2 catch-alls
  **first** and tier-1 specifics **after** them (the block above is ordered
  for readability; the applied file must invert the two sections).
- GitHub cannot express "tier 2 = notify but don't gate" natively when
  branch protection requires code-owner review; if that protection is on,
  keep tier-2 entries out of CODEOWNERS and use a labeler action
  (`.github/labeler.yml` path rules adding `risk:tier-1` / `risk:tier-3`
  labels) for routing instead. The tier labels also give reviewers the
  at-a-glance signal goal 4 asks for.

### (d) Is a `// TRICKY:` comment convention worth adding?

**Yes, narrowly — as a complement, not an alternative, to path tiers.**
Path-based tiers are coarse: `activate.rs` is 1,240 lines, of which perhaps
40 are genuinely dangerous (the exec/fd block at lines 583–597, the nested
`FLOX_ENV` inheritance comment at line 514). A reviewer told "tier 1, be
careful" still has to find the load-bearing lines. Line-level markers fix
that, and the codebase already writes them informally — e.g. the prose
warnings at `activate.rs:514` and `583` and the `.symver` block comment in
`ld-floxlib/ld-floxlib.c:34` are `TRICKY:` comments in everything but name.

Recommended contract (mirroring Rust's `// SAFETY:` discipline):

- A `// TRICKY:` comment must state (i) the invariant, (ii) what observable
  thing breaks if violated, and (iii) where the invariant is enforced or
  tested, if anywhere. "This is subtle" alone is not a valid TRICKY comment.
- Scope it to invariants that **cannot** be encoded in types or caught by
  fast tests — shell quoting rules, GLIBC symbol versioning, fd/exec
  ordering, schema-migration compatibility. If the invariant can be a type
  or a test, do that instead (the `Manifest<S>` type-state pattern is the
  model: it made a whole class of TRICKY comments unnecessary).
- It is grep-able (`grep -rn "TRICKY:" cli/ assets/ ld-floxlib/`), which
  lets review tooling or a PR bot list every touched-TRICKY-region in the
  diff — a finer-grained signal than the path tier.

What it does **not** replace: AGENTS.md cautions (those teach the pattern;
TRICKY marks the instance) and CODEOWNERS (gating must not depend on
authors remembering to write a comment).

### (e) How the target structure makes review intensity path-legible

The target architecture (GOAL.md: commands become `parse → call operation →
render`) turns today's worst review problem — fat command files mixing
tier-3 rendering with tier-2 logic and tier-1 process control in one path —
into a clean mapping:

```
diff touches…                                review tier (from path alone)
cli/flox/src/commands/*           parse+render glue            → 3
operations layer (SDK ±)          typed business logic         → 2
cli/flox-manifest/, schemas/      on-disk contracts            → 1
cli/flox-activations/, shell_gen/,
assets/environment-interpreter/,
ld-floxlib/, auth paths           process/shell/credential     → 1
```

Today a 50-line diff to `install.rs` could be a help-text tweak or a
resolution-logic change; the path cannot tell you which, so every command
diff costs a careful read. After the split, logic changes *cannot* appear
under `commands/` (Workstream D's enforced layering bans the dependencies
that would make it possible), so a `commands/`-only diff is provably glue.
The tier-1 set is unaffected by the refactor — activation, auth, schema,
and native code stay where they are — meaning the CODEOWNERS scheme in (c)
can be adopted **now** and survives the migration; only the
`commands/` tier flips from 2 to 3 when Workstream A's backlog completes.
One refactor-specific risk to encode at that time: the operations layer
becomes the floxhub/floxdash surface, so *public API shape* changes in the
SDK acquire contract-like consequence — add the SDK's public-surface
modules to tier 1 (or gate with a `cargo public-api` CI diff) once external
consumers exist.

---

## Assumptions

- **Shallow history:** the clone has 457 commits back to 2026-03-10
  (`git rev-parse --is-shallow-repository` → `true`), so fix-density covers
  ~3 months, not the requested 17. Conclusions that depend on *absence* of
  churn (ld-floxlib, nix-plugins, lockfile) are stated as
  consequence-based, not frequency-based, for this reason. Re-run the
  measurements on a full clone before treating densities as long-run rates.
- `fix:`-prefix counting assumes conventional commits are used
  consistently (AGENTS.md mandates them); fixes merged under `feat:` or
  `chore:` are undercounted.
- Team names in the proposed CODEOWNERS other than `@flox/cli` and
  `@flox/floxhub` are placeholders; actual GitHub team topology is unknown
  from the repo.
- `cargo`/`nix` were unavailable; no build- or dependency-graph-derived
  evidence is used (that is Workstream D's job).
- The path `assets/activation-scripts/` named in GOAL.md/AGENTS.md is
  assumed to be the same artifact as the existing
  `assets/environment-interpreter/` (renamed before the shallow-history
  cutoff); no commit in available history touches the old path.

## How to reproduce

All commands run from any directory; `-C /home/user/flox` pins the repo.

```sh
# History depth / shallow check / baseline fix ratio
git -C /home/user/flox rev-parse --is-shallow-repository
git -C /home/user/flox log --oneline | wc -l                      # 457
git -C /home/user/flox log --format="%ad" --date=short | tail -1  # 2026-03-10
git -C /home/user/flox log --oneline | grep -cE "^[0-9a-f]+ fix(\(|:|!)"  # 115

# Per-path fix density (the exact loop used)
for p in assets/environment-interpreter ld-floxlib nix-plugins \
  cli/flox-activations cli/flox-manifest cli/flox-manifest/src/lockfile \
  cli/flox-manifest/src/migrate cli/flox-rust-sdk/src/providers/git.rs \
  cli/flox-rust-sdk/src/providers/git_auth.rs \
  cli/flox-rust-sdk/src/providers/nix_auth.rs cli/flox-catalog/src/auth \
  cli/flox-catalog/src/token.rs cli/flox/src/commands/auth.rs \
  cli/flox/src/commands/activate.rs cli/flox-rust-sdk/src/models \
  cli/flox/src/commands cli/flox-rust-sdk/src/providers/buildenv.rs \
  cli/flox-rust-sdk/src/providers/publish.rs cli/flox-core; do
  total=$(git -C /home/user/flox log --oneline --since="2025-01-01" -- "$p" | wc -l)
  fixes=$(git -C /home/user/flox log --oneline --since="2025-01-01" -- "$p" \
          | grep -cE "^[0-9a-f]+ fix(\(|:|!)")
  echo "$p: total=$total fix=$fixes"
done

# Sample fix subjects cited in the risk map
git -C /home/user/flox log --oneline -- cli/flox-activations | grep -E " fix(\(|:|!)"
git -C /home/user/flox log --oneline -- cli/flox-rust-sdk/src/providers/publish.rs | grep -E " fix(\(|:|!)"
git -C /home/user/flox log --oneline -- assets/environment-interpreter | grep -E " fix(\(|:|!)"

# Structural evidence
cat /home/user/flox/.github/CODEOWNERS                 # single line: /cli/schemas/
head -80 /home/user/flox/ld-floxlib/ld-floxlib.c       # .symver blocks, both arches
grep -n "exec\|inherit" /home/user/flox/cli/flox/src/commands/activate.rs  # lines 514, 583, 597
grep -n "GitRemoteCommandError\|AccessDenied\|Diverged" \
  /home/user/flox/cli/flox-rust-sdk/src/providers/git.rs                   # lines 690-815
ls /home/user/flox/cli/schemas/                        # the three published schemas
ls /home/user/flox/assets/                             # environment-interpreter (not activation-scripts)
wc -l /home/user/flox/cli/flox/src/commands/activate.rs                    # 1240
```
