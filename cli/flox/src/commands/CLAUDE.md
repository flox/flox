# Conventions for `cli/flox/src/commands/`

Reviewers in this area most often catch three categories of problem: (1) typed domain values being passed as raw strings — especially flake references, shell types, and URLs — when purpose-built types already exist; (2) user-facing messages that are inaccurate, noisy, or missing actionable next steps; and (3) test coverage that either tests bpaf plumbing instead of behavior, or is missing entirely for new command paths. A smaller but steady stream of comments addresses incomplete `ConcreteEnvironment` match arms, redundant parsing of already-parsed values, and helper functions whose names describe how they were triggered rather than what they do.

## Area-specific rules

### Type safety at CLI boundaries

**Use `NixFlakeRef` (or equivalent typed flake-reference type) for any field that holds a Nix flake reference; parse it at the CLI arg-parsing boundary and propagate the typed value.**
Raw strings force every downstream consumer to parse and validate independently, producing brittle string-splitting logic such as `.split("?rev=").nth(1)`. The typed accessor (e.g. `.rev()`) replaces that.
Evidence: PR #3599, PR #4156

**Default to `COMMON_NIXPKGS_URL` (the project-wide constant) rather than the bare string `"nixpkgs"` when constructing a fallback flake reference.**
Using a bare `"nixpkgs"` string resolves to whichever nixpkgs is on the user's registry, which may differ from the pinned nixpkgs Flox uses for builds and evaluation.
Evidence: PR #3599

**Use the `Shell` enum (from `shell_gen`) rather than a plain `String` when a CLI argument represents a shell name.**
An untyped string reaches a `_ => {}` wildcard arm and silently does nothing for unsupported shells; the enum turns that into a compile-time exhaustiveness check.
Evidence: PR #4231

**Parse URLs with `url::Url` at CLI boundaries and update downstream consumers to use typed accessors.**
Parsing a URL string deep inside command logic forces re-parsing at each call site and prevents the type system from guaranteeing well-formedness.
Evidence: PR #4156

**Access manifest descriptor fields directly instead of reconstructing equivalent data from the locked/catalog entry.**
`PackageToList::Catalog` carries both the manifest descriptor and the locked entry. When the descriptor already has the authoritative value (e.g. `pkg_path`), use it directly rather than reconstructing from catalog fields.
Evidence: PR #3700

### Error handling in command implementations

**Match all `ConcreteEnvironment` variants (Path, Managed, Remote) when gating a feature to a subset; do not silently exclude variants.**
A `match` that only handles `Path` and bails on `Managed` with "not supported" may be wrong — check whether the operation actually makes sense for the other variants before blocking them.
Evidence: PR #3599

**Remove obsolete string-matching branches after introducing a typed error variant.**
When a new enum variant (e.g. `ManagedEnvironmentError::UpstreamAlreadyExists`) supersedes a string-match guard, delete the old branch; leaving it produces dead code that diverges over time.
Evidence: PR #3646

**Distinguish auth context in user-facing messages: do not say "not logged in" when the user is authenticated via Kerberos.**
`flox auth status` reporting "You are not currently logged in to FloxHub" is inaccurate when `AuthContext` is `Krb`. The message must reflect the actual auth state. Add a `TODO` tracking the full Kerberos handling if deferring.
Evidence: PR #4172

**Add a `TODO` comment (with ticket reference where possible) when an auth flow is undefined for a given configuration mode; do not silently fall through.**
`flox auth login` invoked under a Kerberos configuration has no meaningful effect. Mark the gap explicitly rather than letting the code succeed silently.
Evidence: PR #4047

**Surface specific error conditions at call boundaries with `.context()`; do not use `.unwrap()` in command logic.**
Commands run in user-facing paths. An `.unwrap()` panic produces an unhelpful backtrace; `.context("Failed to …")` attaches the surrounding operation so the user and the error chain both understand what went wrong.
Evidence: PR #4096

### CLI flag placement and bpaf usage

**Place flags that modify the overall command at the top-level struct rather than duplicating them inside each variant of a nested enum.**
Positional ambiguity grows when flags appear only on some variants; hoisting shared flags to the parent struct eliminates the ambiguity and simplifies help text.
Evidence: PR #3715

**Name positional arguments by their actual type (`installable`, `attrpath`), not by an approximate concept (`expression`, `package-name`).**
The argument name appears in `--help` and shell completions. Users familiar with Nix expect `installable` to mean `[flake-ref#]attr-path`; `expression` implies Nix language syntax, which is wrong here.
Evidence: PR #3599

**Prefix intentionally unused function parameters with `_` (e.g. `_flox`).**
The compiler warning confirms the parameter is not accidentally forgotten. Reviewers will flag unnamed unused parameters as a readability issue.
Evidence: PR #4219

**Rename functions to describe what they do, not what triggered them.**
A function named after its trigger (e.g. `gather_services_for_flag`) misleads callers about its actual contract. Use a name that describes the computation (e.g. `services_to_start`).
Evidence: PR #4152

### Command output and user-facing messages

**Use the `message::` helpers (`message::updated`, `message::warning`, `message::plain`, etc.) for all terminal output; do not `println!` directly.**
The helpers enforce consistent formatting and the correct emoji/icon for the message kind (see AGENTS.md for the standard emoji map).
Evidence: PR #3902

**Emit warnings only for conditions that require user action; suppress noise for outcomes triggered automatically.**
If Flox auto-started services and the result is expected, no warning is needed. Warnings are for situations where the user must do something differently next time.
Evidence: PR #4152

**Keep messages concise when the state has not changed; avoid verbose "nothing to do" phrasing.**
Saying "Environment is already up to date. No packages were upgraded." repeats itself. One short sentence suffices.
Evidence: PR #3869

**Eliminate redundant warnings; use terminology that is accurate for all contexts, not just the current tool.**
A message that copies tool-specific language (e.g. "nix build failed") surfaces internals. Rewrite at the product level and apply consistently to every build back-end.
Evidence: PR #4156

**Rewrite copy-pasted error messages to describe the actual operation, not the operation that was copied from.**
Copy-paste errors are common when adding new commands. Always verify each message is accurate for its specific context before submitting.
Evidence: PR #3969

**Provide actionable next steps in error messages when users encounter a restriction.**
"Cannot do X in a managed environment." leaves users stuck. Follow it with "Use `flox edit` to modify the environment locally, then push."
Evidence: PR #3649

**Use precise terminology: prefer `targets` over `artifacts` when store paths are unavailable; prefer generic `build` over tool-specific terms in user messages.**
Precision matters when users paste messages into bug reports or search for help. Inaccurate vocabulary creates a vocabulary mismatch.
Evidence: PR #4232, PR #4156

### Control flow and implementation structure

**Use `tokio::select!` to wait for either a signal handler or CLI completion, ensuring the temp directory and guards are dropped on interrupt.**
Signal handlers must race against the CLI worker so that `Ctrl-C` triggers cleanup (dropping `temp_dir`, metrics guards, sentry). Blocking on the CLI future alone leaves cleanup uncalled on interrupt.
Evidence: PR #3600

**Apply early-return guards with `let Some(x) = y && condition` to skip expensive operations when they are unnecessary.**
Reading generation metadata to compare a generation number is wasteful when no generation was requested. Short-circuit before the metadata read.
Evidence: PR #3715

**Refactor duplicated logic into a single unified function to prevent message inconsistencies across code paths.**
When the same service-start logic appears in two branches with slightly different messages, the branches diverge over time. Extract to one function; adjust the message at the call site if needed.
Evidence: PR #4152

**Clone shared data at the ownership boundary, not throughout the function body.**
A clone that appears inside a conditional or repeated in several arms should be hoisted to the point where ownership transfers, making the intent explicit.
Evidence: PR #4172

**Remove wrapper methods that simply delegate to a field's method.**
`Flox::get_handle()` wrapping `flox.auth_context.handle()` adds indirection with no benefit. Callers should access the field method directly.
Evidence: PR #4172

**Use `nix_expression_dir(&env)` and other existing path helpers instead of re-constructing paths from `env.parent_path()` manually.**
Constructing `.flox/pkgs/` by hand diverges from the canonical path if the helper's logic changes. Always prefer the established helper.
Evidence: PR #3599

**When writing file content fetched as bytes, use `fs::write` directly on the byte slice; do not round-trip through `String`.**
Converting bytes to `String` to write them back to disk adds a UTF-8 validation that is unnecessary for binary or opaque content and silently drops non-UTF-8 data.
Evidence: PR #3599

### Testing in command implementations

**Write unit tests for formatting functions by accepting `&mut impl Write` so tests can write to a `Vec<u8>` buffer.**
Formatting logic that writes directly to stdout cannot be unit-tested without capturing output. Accepting a writer makes the function testable without subprocess overhead.
Evidence: PR #3695

**Add integration (bats) tests that verify real workflows end-to-end for new commands, not only unit mocks.**
A new command that only has unit tests may pass CI while being broken in practice. At minimum, add a happy-path integration test that exercises the full CLI stack.
Evidence: PR #3969

**Remove trivial tests that only verify bpaf parser mechanics (e.g., that a flag parses to `true`).**
bpaf is a library with its own test suite. Testing that `--force` sets `force: true` adds no value; test what the command *does* with the flag.
Evidence: PR #4200

**After changing shell completion output, verify completions work end-to-end in the actual shell, not only by checking static `--bpaf-complete*` strings.**
Static completion strings can be correct while the shell integration is broken. Provide a concrete test command (e.g. `flox activate -- fzf<TAB>`) that exercises the live completion path.
Evidence: PR #3988

**Use `mock_managed_environment_in` for managed environment tests and extract environment-existence checks into a shared helper.**
Copy-pasting setup boilerplate into multiple tests causes subtle divergence. The shared helper is the canonical setup.
Evidence: PR #3599

## Cross-cutting reminders

These cross-cutting themes hit this area often — see the parent skills for the full text:

- Error type hierarchy (extend enum variants; do not string-match on `.to_string()`) — `.claude/skills/flox-rust-review/SKILL.md`
- `use` statement placement (module scope, not function scope; update when moving code) — `.claude/skills/flox-rust-stylistic-conventions/SKILL.md`
- `formatdoc!` / `indoc!` for multiline strings (not `format!` with `r#"..."#`) — `.claude/skills/flox-rust-stylistic-conventions/SKILL.md`
- User-visible message structure (sentence case, no subject "flox/we/I", one sentence per line, actionable last line) — `.claude/skills/flox-rust-review/SKILL.md`
- Test naming (no `test_` prefix; descriptive names that state what is verified) — `.claude/skills/flox-rust-stylistic-conventions/SKILL.md`
- Manifest type-state (`Manifest<S>` constructors; never deserialize inner types directly) — `.claude/skills/flox-rust-review/SKILL.md`

## When in doubt

The most active reviewers in this area are `ysndr` (Tier 1) and `dcarley` (Tier 1). Their prior comments are the best precedent — when uncertain, search PR history for similar work in `cli/flox/src/commands/`. `mkenigs` (Tier 1) contributes heavily on `activate.rs` and service-related files.
