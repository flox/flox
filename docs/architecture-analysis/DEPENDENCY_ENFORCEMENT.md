# Dependency Enforcement — Recommended Mix for Flox

Date: 2026-06-11
Plan: companion to `D-dependency-layering.md` (which proposes the layering
policy this document operationalizes) and `REPORT.md` §4 decision #2.
Status: analysis only — the configs below are proposals to be committed in a
future implementation PR, not from this document.

Recommendation, grounded in what the analysis found: use **four mechanisms,
each covering a failure mode the others can't see**, plus one to add later.
Flox already has the scaffolding for three of them (an `xtask` crate, a
`cli/clippy.toml`, CI jobs running clippy).

## The mix

| Failure mode | Mechanism | Catches it where |
|---|---|---|
| Terminal/UI crate sneaks below the binary layer | **cargo-deny `[bans]`** | dependency graph, CI |
| Workspace edge points upward (L1 crate depends on the SDK); new crate added with no layer decision | **`xtask lint-layers`** over `cargo metadata` | dependency graph, CI |
| Code below L3 prints, probes the terminal, or reads ambient state — with no new dependency to flag | **clippy `disallowed_*`** per-crate | source code, CI |
| CLI reaches into SDK plumbing it shouldn't (and vice versa) | **visibility tightening** (`pub(crate)`) | compiler, instantly |
| Risky change merges without the right eyes | **CODEOWNERS tiers** (from `F-risk-map.md`) | review process |

Don't use feature flags for this (Cargo features unify across the workspace,
so one enabler re-contaminates everyone), and don't rely on AGENTS.md prose
alone — that's documentation, not enforcement.

## 1. cargo-deny — terminal crates are direct deps of binaries only

New file `deny.toml` at the workspace root (this is
`D-dependency-layering.md`'s draft; it fails on today's tree until the
`flox-core` fix lands, which is the point — adopt both in one PR):

```toml
[bans]
multiple-versions = "warn"
deny = [
    # Terminal/UI crates: only the presentation binaries may name them.
    { name = "crossterm",         wrappers = ["flox"] },
    { name = "supports-color",    wrappers = ["flox"] },
    { name = "inquire",           wrappers = ["flox"] },
    { name = "tracing-indicatif", wrappers = ["flox"] },
    { name = "minus",             wrappers = ["flox"] },
    { name = "indicatif",         wrappers = ["flox", "mk_data"] },
    # Crates nothing in this workspace should ever add:
    { name = "dialoguer" },
    { name = "console" },
    { name = "termion" },
]
```

CI: `cargo deny check bans` in the same job that runs clippy. Zero custom
code, and it catches *transitive* sneak-ins (a new dependency that itself
pulls crossterm).

## 2. xtask lint-layers — direction rules cargo-deny can't express

cargo-deny bans crates globally; it can't say "reqwest is fine in the SDK but
banned in flox-core" or "no L1 crate may depend on L2." A ~80-line subcommand
in the existing `cli/xtask` crate can:

```rust
use cargo_metadata::MetadataCommand;
use std::collections::HashMap;

pub fn lint_layers() -> anyhow::Result<()> {
    let layers: HashMap<&str, u8> = HashMap::from([
        ("flox", 3), ("flox-activations", 3), ("mk_data", 3), ("xtask", 3),
        ("flox-rust-sdk", 2), ("beta", 2),
        ("flox-catalog", 1), ("flox-manifest", 1),
        ("nef-lock-catalog", 1), ("catalog-api-v1", 1),
        ("flox-core", 0), ("shell_gen", 0), ("systemd", 0), ("flox-events", 0),
    ]);
    // Deps banned at or below a given layer (terminal crates handled by cargo-deny):
    let banned_at: HashMap<u8, &[&str]> = HashMap::from([
        (0u8, &["reqwest", "sentry", "sysinfo"][..]),  // L0 is context-free
    ]);

    let meta = MetadataCommand::new().exec()?;
    let mut errors = vec![];
    for pkg in meta.workspace_packages() {
        let Some(&layer) = layers.get(pkg.name.as_str()) else {
            errors.push(format!("{}: new crate has no layer assignment — add it to lint_layers", pkg.name));
            continue;
        };
        for dep in pkg.dependencies.iter().filter(|d| d.kind == cargo_metadata::DependencyKind::Normal) {
            if let Some(&dep_layer) = layers.get(dep.name.as_str()) {
                if dep_layer > layer {
                    errors.push(format!("{} (L{layer}) depends on {} (L{dep_layer}): upward edge", pkg.name, dep.name));
                }
            }
            if banned_at.get(&layer).is_some_and(|b| b.contains(&dep.name.as_str())) {
                errors.push(format!("{} (L{layer}): banned dependency {}", pkg.name, dep.name));
            }
        }
    }
    if errors.is_empty() { Ok(()) } else { anyhow::bail!(errors.join("\n")) }
}
```

CI: `cargo run -p xtask -- lint-layers`. The "no layer assignment" error is
the quiet superpower: every future crate forces an explicit architecture
decision at PR time instead of drifting in unanchored (exactly what happened
to `flox-events`, which has zero consumers today —
`D-dependency-layering.md` violation #6).

## 3. Clippy disallowed-lists — in-code violations no graph can see

A crate can stay dependency-clean and still `println!` or read `std::env`
ambiently. Per-crate `clippy.toml` files in the library crates
(`cli/flox-rust-sdk/clippy.toml`, `cli/flox-core/clippy.toml`, etc.):

```toml
disallowed-macros = [
    { path = "std::println",  reason = "library crates return data; binaries render it" },
    { path = "std::eprintln", reason = "library crates return data; binaries render it" },
]
disallowed-methods = [
    { path = "std::env::current_dir", reason = "take the path as a parameter (see B-sdk-fitness.md R10)" },
]
```

This mechanizes `D-dependency-layering.md`'s rule 3 and would have flagged
the SDK's one ambient cwd read (`remote_environment.rs:469`) automatically.
Note: `disallowed-methods` on `std::env::var` is probably too aggressive for
the SDK today (the ~20 tool-path overrides like `NIX_BIN` are legitimate —
`B-sdk-fitness.md` Pass 1) — start with prints and `current_dir`, expand as
the R-list lands. Test code is unaffected if you `#[allow]` in
`#[cfg(test)]` modules.

## 4. Visibility — let the compiler enforce the API surface

Cheapest of all, no tooling: narrow what the SDK exports so illegal coupling
won't compile. Concrete first candidates from `B-sdk-fitness.md`:
`nix::nix_base_command` → `pub(crate)` once the stranded `gc.rs` store-gc
logic moves into the SDK (R11/A17), and the `utils` grab-bag module →
unexported. This doubles as the experiment B's verdict hinges on: if
tightening visibility breaks the CLI in surprising places, that reveals
load-bearing coupling and is evidence *for* a facade crate.

## 5. CODEOWNERS — the human layer

Adopt `F-risk-map.md`'s tier-1 scheme now (it survives the migration
unchanged). It's the only mechanism on this list that enforces *review
intensity* rather than code structure, and it closes the gap F found —
`cli/flox-manifest/src/parsed/` can change the schema contract today without
touching the one protected path.

## Later, not now

Once floxhub/floxdash actually link the SDK, add a **`cargo public-api` diff
gate** on `flox-rust-sdk` so public-surface changes become explicit, reviewed
events (`F-risk-map.md` §(e) flags this as the one new tier-1 risk the
refactor creates). Premature today — the surface is still being deliberately
reshaped.

## Sequencing

All four code mechanisms land in **one PR together with the `flox-core`
crossterm fix** (Phase 0 of the `REPORT.md` backlog): the `deny.toml` and
clippy configs are written to fail on the pre-fix tree, so merging them
together means the clean state is born enforced and can never silently
regress. Total cost is roughly a day of work, most of it the xtask lint.

Implementing Phase 0 — the crossterm fix plus all four enforcement
mechanisms — is the first concrete step out of analysis-only territory, and
should be a separate PR from the analysis docs.
