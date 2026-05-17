# AGENTS.md Gap Report
## Proposed Amendments Based on PR Review Evidence

---

## Method

This report proposes amendments to AGENTS.md based on patterns found in 944 review comments across 216 pull requests spanning approximately 8 months. The analysis extracted 67 findings where `in_agents_md=0` and `confidence_score >= 0.5`. All findings are reported here.

**Critical structural caveat: every single finding in this dataset has `total_evidence_count=1`.** No rule is backed by more than one comment. This means none of the proposed amendments are based on repeated, independently confirmed patterns. The signal must therefore be treated as "a reviewer raised this at least once" rather than "reviewers consistently enforce this." Two tiers of confidence are used throughout:

- **T1-accepted** (`tier1_reviewer_count=1` AND `acceptance_rate=1.0`): A Tier-1 reviewer raised the issue and the author accepted it. Stronger signal. Still single-evidence.
- **T1-raised** (`tier1_reviewer_count=1` AND `acceptance_rate=null` or `acceptance_rate=0.0`): A Tier-1 reviewer raised it; outcome is unknown or the suggestion was not adopted. Weaker signal — flag for more evidence before adopting.
- **T2-only** (`tier1_reviewer_count=0`): Only Tier-2 reviewers raised the issue. Lowest signal.

Findings were grouped thematically. Where a proposed rule is a sharper or more specific version of an existing AGENTS.md guideline, the amendment is marked **EXPANSION**. New entries are marked **NEW**.

Tier-1 reviewers: **ysndr**, **mkenigs**, **dcarley**.
Tier-2 reviewers: gilmishal, billlevine, djsauble.

---

## Section 1: Proposed Amendments

Amendments are grouped by theme and ranked within each group. The most actionable (T1-accepted) rules lead each group.

---

### Group A: Rust Code Clarity — Constants, Comments, and Labelled Blocks

**A1. [NEW] Use named constants instead of magic numbers** (confidence: T1-accepted, PR #3801)

Reviewers flagged bare integer literals where named constants exist in the same scope or a well-known dependency.

Suggested AGENTS.md text to add under "Rust style":
```
  - **No magic numbers:** Use named constants rather than bare integer literals.
    For POSIX file descriptors use `nix::libc::STDERR_FILENO`, `STDOUT_FILENO`,
    `STDIN_FILENO` rather than `2`, `1`, `0`.
```

Evidence: PR #3801 — mkenigs: "nit: we could use nix::libc::STDERR_FILENO"

---

**A2. [EXPANSION] Place comments adjacent to the code they document** (confidence: T1-accepted, PR #3770)

AGENTS.md already says to use structured fields in tracing. This extends guidance to ordinary code comments.

Suggested addition under "Rust style":
```
  - Place comments immediately above the line or block they explain.
    Don't separate a comment from its target by blank lines or unrelated code.
```

Evidence: PR #3770 — mkenigs: "this comment got relocated in the wrong spot and would be more helpful on the generate functions"

---

**A3. [NEW] Avoid labelled blocks when a simple `if` suffices** (confidence: T1-raised, PR #4045)

Reviewers noted that labelled blocks (`'label: { ... }`) require the reader to mentally jump backwards through control flow.

Suggested AGENTS.md text:
```
  - Prefer `if condition { body }` over labelled blocks (`'label: { ... break 'label; }`)
    when the only purpose of the label is to exit the block early. The label requires
    the reader to reverse-trace flow; a direct conditional is clearer.
```

Evidence: PR #4045 — dcarley: "The labelled block seems odd and requires you to mentally jump backwards through the logic"

NOTE: acceptance_rate=null; needs more evidence before adopting.

---

**A4. [NEW] Avoid unnecessary clone allocations** (confidence: T2-only, PR #4172)

When a function signature allows using a reference, avoid cloning the value. This was noted in the context of authentication token handling.

Suggested AGENTS.md text:
```
  - Avoid unnecessary clones: before cloning a value to pass to a function,
    check whether the function signature can accept a reference instead.
```

Evidence: PR #4172 — gilmishal (T2)

NOTE: T2-only, acceptance_rate=1.0. Reasonable general guidance but lowest confidence in this dataset.

---

### Group B: Rust Code Clarity — Enum Naming

**B1. [NEW] Use singular form for enum variants** (confidence: T1-accepted, PR #3870)

AGENTS.md has naming guidance for helpers (`str_to_x`, `with_x`) but does not address enum variant naming conventions.

Suggested AGENTS.md text to add under "Rust style":
```
  - **Enum variants:** Use singular form for variant names (e.g., `Auth0` not `Auth0s`,
    `AuthStrategy::Auth0` not `AuthStrategy::Auth0s`). This is consistent with Rust
    standard library conventions.
```

Evidence: PR #3870 — mkenigs: "nit: I think we use singular for most of our enums"

---

**B2. [NEW] Distinguish config-level types from runtime-material types in naming** (confidence: T2-only, PR #4172)

The rename from `AuthMethod` to `AuthnMode` (config) vs `AuthContext` (runtime) reflects a semantic distinction: mode describes configuration intent; context describes what is available at runtime.

Suggested AGENTS.md text:
```
  - When a concept has both a configuration-time and runtime-material form, choose
    names that reflect the lifecycle: use `Mode` or `Config` for the configuration
    type and `Context` or `State` for the runtime type. Document this distinction
    in the module's `//!` doc comment.
```

Evidence: PR #4172 — gilmishal (T2)

NOTE: T2-only; useful design-level guidance but needs more evidence.

---

### Group C: Rust Code Clarity — Use Statements and Cargo Dependencies

**C1. [EXPANSION] Always use workspace dependency versions in Cargo.toml** (confidence: T1-accepted, PR #3939)

AGENTS.md already has `use` import guidance but does not cover Cargo.toml dependency management.

Suggested addition under "Rust style":
```
  - **Cargo dependencies:** Always declare crate dependencies in `Cargo.toml` using
    `dep.workspace = true` rather than inline version strings. Adding a version inline
    (e.g., `temp-env = "0.3"`) when a workspace entry already exists produces a noisier
    `Cargo.lock` diff and can introduce version divergence.
```

Evidence: PR #3939 — dcarley: "We already have a version of this in the workspace, which is possibly why the Cargo.lock change is more noisy than I'd expected."

---

### Group D: Error Handling — Expired Tokens and Graceful Degradation

**D1. [NEW] Send expired tokens for identification rather than an empty string** (confidence: T1-accepted, PR #3921)

When a token is expired, using the expired token (rather than `""` or omitting it) still lets the server log *who* attempted authentication, which aids debugging.

Suggested AGENTS.md text to add under "Error handling architecture":
```
  - **Expired credentials:** Pass expired tokens to downstream services rather than
    replacing them with empty strings. An expired token still conveys the identity
    of the requester, which helps with server-side logging. Document the expired state
    with a debug log entry before using the token.
```

Evidence: PR #3921 — ysndr: "I think its better to have something in the shape of a token than a sentinel `""` even if the token is expired. If only because it will tell FloxHub who tries to authenticate"

---

**D2. [NEW] Add diagnostic messages for unsupported auth modes on incompatible builds** (confidence: T1-accepted, PR #4172)

When a runtime configuration requests a feature (e.g., Kerberos) that is not compiled into the current binary, surface a user-visible error or warning rather than silently falling through.

Suggested AGENTS.md text:
```
  - **Unsupported build features:** When a user-configured mode or feature is not
    compiled into the current binary (e.g., Kerberos auth without the
    `floxhub-authn-kerberos` feature flag), emit a diagnostic message rather than
    silently using a fallback or doing nothing. Track missing implementations with
    a `// TODO(<issue>):` comment.
```

Evidence: PR #4172 — ysndr: "nit: i think we should have a warning/error case for use of the kerberos mode on non kerberos-enabled installations"

---

### Group E: User-Facing Messages — Documentation and Man Pages

**E1. [NEW] Mark unstable JSON outputs explicitly in man pages** (confidence: T1-raised, PR #3651)

When a `--json` flag is added to a command, the man page should explicitly state that the output schema is not guaranteed to be stable across releases.

Suggested AGENTS.md text to add under "User-visible message syntax, structure, and content":
```
  - **Unstable output formats:** When documenting `--json` or other machine-readable
    output flags in man pages, explicitly state stability: "Attention: the output
    format is not guaranteed to be stable and may change across releases of Flox."
```

Evidence: PR #3651 — ysndr: "How should we mark this more explicitly as potentially unstable?"

NOTE: acceptance_rate=0.0 (suggestion was not adopted in that PR); flag for more evidence.

---

**E2. [NEW] Use mutually exclusive notation in man page SYNOPSIS for conflicting flags** (confidence: T1-raised, PR #3651)

When two flags are mutually exclusive, use `--flag1 | --flag2` notation in the SYNOPSIS section, not adjacent short options that look like aliases.

Suggested AGENTS.md text:
```
  - **Man page SYNOPSIS notation:** When two options are mutually exclusive, write
    `[-t | --json]` in the SYNOPSIS rather than listing them adjacently. The pipe
    operator signals that only one may be chosen.
```

Evidence: PR #3651 — mkenigs: "Might be better to have `--tree | --json` in the man page"

NOTE: acceptance_rate=0.0; this was adopted in the final_code_snippet but not flagged as accepted at review time. Treat as weak signal.

---

**E3. [NEW] Add man page reference or TODO when a feature-flagged subcommand is added** (confidence: T1-accepted, PR #3969)

When a subcommand is added behind a feature flag (e.g., `#[bpaf(hide)]`), include either a man page or a `// TODO: add man-pages when un-hiding this` comment.

Suggested AGENTS.md text:
```
  - **Hidden subcommands and man pages:** When adding a subcommand behind a feature
    flag with `#[bpaf(hide)]`, either include the man page immediately or add a
    `// TODO: add man-pages when we un-hide this` comment. The `footer(...)` attribute
    should reference an existing man page only.
```

Evidence: PR #3969 — mkenigs: "looks like we need to actually add the man page? Or add a `// TODO` for when we flip the feature flag?"

---

**E4. [EXPANSION] Use precise domain terminology in user-facing messages** (confidence: T1-accepted, PR #4232)

AGENTS.md already has guidance on brand naming but not on distinguishing Nix-specific terminology ("targets" vs "artifacts").

Suggested AGENTS.md text to add under "User-visible message syntax":
```
  - Use precise Nix terminology in output: "targets" refers to named build outputs
    identified in the manifest; "artifacts" implies built file paths that the user
    can examine. Use "targets" when paths are not available, "outputs" or "artifacts"
    when reporting concrete file paths.
```

Evidence: PR #4232 — dcarley: '"artifacts" sounds like they should be paths... but we don't have the paths available so we could just say that these are targets'

---

**E5. [NEW] Frame breaking changes as user benefits in release communication** (confidence: T1-raised, PR #3803)

When a command's behavior changes in a breaking way, the PR description or release notes should articulate what the user gains, not just what they lose.

Suggested AGENTS.md text:
```
  - **Breaking changes:** When changing the behavior of an existing command, explain
    in the PR description what the user benefits from the new behavior — don't only
    describe what no longer works. This guides changelog authors and release communication.
```

Evidence: PR #3803 — dcarley: "Is there anything we can say to sell this as a benefit to users?"

NOTE: This is a PR-process guideline, not a code guideline. acceptance_rate=1.0 but the scope is PR authorship. Needs more evidence before adding to AGENTS.md.

---

### Group F: Code Comments — Documenting Edge Cases and Deferred Work

**F1. [EXPANSION] Document edge cases in inline comments with supporting evidence** (confidence: T1-accepted, PR #4215)

AGENTS.md mentions understanding semantics before rewriting error messages. This extends the principle to documenting *why* an edge case is handled (or deliberately not handled) in code.

Suggested AGENTS.md text to add under "Rust style":
```
  - **Documenting edge cases:** When code deliberately handles (or skips) an edge case
    because it is rare, document the reasoning inline. If the rarity depends on
    upstream behavior (e.g., how nixpkgs stdenv populates `meta.outputsToInstall`),
    include a brief citation or Nix snippet so future readers can verify the assumption
    without reading external sources.
```

Evidence: PR #4215 — dcarley: "worth clarifying in the comment, which would help us in the future and would likely have guided the code review"

---

**F2. [NEW] Document deferred work with tracking issues, not just TODO comments** (confidence: T1-accepted, PR #3801)

When a reviewer asks about a missing feature (e.g., a warning for Kerberos on non-Kerberos builds), and the decision is to defer, record the deferral in a tracking issue and reference it in a `// TODO(<issue>):` comment.

Suggested AGENTS.md text:
```
  - **Deferred work:** When deferring an improvement to a follow-up, create a
    tracking issue and annotate the code with `// TODO(<issue>): description`.
    A bare `// TODO` without a ticket is harder to prioritize and may be forgotten.
```

Evidence: PR #3801 — dcarley: "I'll add it to the tracking issue and follow-up"
Evidence: PR #4172 — gilmishal: "Added TODO(ENT-105) and created a follow-up issue"

---

**F3. [EXPANSION] Preserve documentation comments when refactoring** (confidence: T1-accepted, PR #3785)

AGENTS.md has guidance on understanding semantics before rewriting. This adds an explicit rule about not silently deleting useful doc comments during refactors.

Suggested AGENTS.md text:
```
  - **Preserving doc comments:** When refactoring or reorganizing code, do not silently
    remove `///` or `//` comments that explain non-obvious behavior. Either move them
    to the new home, rewrite them to match the new structure, or add a replacement
    that captures the same intent.
```

Evidence: PR #3785 — ysndr: "i think we're losing useful documentation by removing these kind of doc comments with no replacement"

---

**F4. [NEW] Document race conditions and constraint assumptions in comments** (confidence: T1-accepted, PR #3920)

When code is known to have a race condition or a constraint that is accepted as a deliberate trade-off, document it inline.

Suggested AGENTS.md text:
```
  - **Known races and accepted limitations:** When code has a known race condition
    or constraint that is accepted without a fix (e.g., "there is a window where
    process-compose could restart with a different store path"), document it with
    an inline comment explaining the constraint and why it is acceptable.
```

Evidence: PR #3920 — dcarley: "I suspect there's room for more race conditions here... there doesn't seem much that we can do about it."

---

**F5. [NEW] Document upstream issues with references rather than forking the fix** (confidence: T1-accepted, PR #3988)

When a workaround exists for an upstream library bug, cite the upstream issue in the doc comment and prefer filing an upstream PR over maintaining a local patch.

Suggested AGENTS.md text:
```
  - **Upstream workarounds:** When working around an upstream library bug, cite the
    upstream issue URL in the doc comment (e.g., `/// Workaround for <url>`). Prefer
    filing an upstream PR or issue rather than maintaining the fix locally, and note
    this intent in the comment.
```

Evidence: PR #3988 — dcarley: "I'd be inclined to not fix it as part of this PR and instead file an upstream PR or issue"
Final code: the accepted snippet added `/// Workaround for <https://github.com/pacak/bpaf/issues/440>`.

---

### Group G: Shell Script Clarity

**G1. [NEW] Avoid double negatives in shell scripts; use positive assertions** (confidence: T1-accepted, PR #3932)

Shell scripts in `assets/` use condition flags. When a flag represents a "skip" or "no-X" concept, the resulting condition `[ "$_no_hook_on_activate" != "true" ]` is a double negative. Rename to a positive form or invert the comparison.

Suggested AGENTS.md text (add under Conventions or a new "Shell script conventions" subsection):
```
  - **Shell script boolean variables:** Prefer positive-assertion variable names over
    negations (e.g., `_run_hook_on_activate="true"` rather than
    `_skip_hook_on_activate="false"`). When reading a negative-named variable, prefer
    the positive comparison (`[ "$_skip_hook_on_activate" = "false" ]`) over double-
    negation (`[ "$_no_hook_on_activate" != "true" ]`).
```

Evidence: PR #3932 — dcarley: "The double negative had me re-read this a few times."

---

### Group H: Testing Conventions

**H1. [NEW] Write unit tests for bug fixes and cross-component interactions** (confidence: T1-raised, PR #3869)

Reviewers repeatedly asked for test coverage when PRs fixed bugs without accompanying tests. AGENTS.md has no guidance on when tests are expected.

Suggested AGENTS.md text (new subsection under Testing):
```
### Test expectations

- **Bug fixes:** Every bug fix should be accompanied by a unit test that reproduces
  the bug and verifies the fix, unless the setup cost is prohibitive. If a test is
  not added, note the gap in the PR description.
- **Cross-component interactions:** When a change affects the interaction between two
  subsystems (e.g., `push` invalidating an upgrade notification for a different
  environment type), prefer a unit test over relying solely on integration tests.
```

Evidence: PR #3869 — mkenigs: "I think we probably want some test coverage for some of the bugs this is fixing"; acceptance_rate=0.0.
Evidence: PR #3869 — ysndr (same PR, upgrade notification TODO).

NOTE: Both supporting comments are from PRs where the suggestion was not adopted. Needs more evidence.

---

**H2. [NEW] Document manual testing steps for tty-dependent behavior** (confidence: T1-raised, PR #3672)

When tty-dependent behavior cannot be automatically tested (color output, pager detection), note in the PR what manual testing was done.

Suggested AGENTS.md text:
```
  - **TTY-dependent behavior:** When adding or modifying behavior that depends on
    whether stdout is a terminal (color output, pager, interactive prompts), document
    in the PR description what manual testing was performed, since automated tests
    cannot easily simulate tty detection.
```

Evidence: PR #3672 — mkenigs: "I tried to test things this will change manually since it's hard to test tty dependent stuff"; acceptance_rate=null.

---

**H3. [NEW] Keep unit tests focused; split oversized functionality to reduce side-effect coverage burden** (confidence: T1-raised, PR #3903)

When a function has too many side-effects to unit-test, that is a signal to refactor the function, not to skip tests.

Suggested AGENTS.md text:
```
  - **Testability as a design signal:** When a function is difficult to unit-test
    because of excessive side-effects, treat this as a design signal: the function
    may be doing too much. Consider extracting pure logic before adding tests, rather
    than relying on integration tests for all coverage.
```

Evidence: PR #3903 — dcarley: "There are a bunch of side-effects to observe, like whether it started an executive, which might hint that it's doing too much"; acceptance_rate=0.0.

---

### Group I: PR Scope Discipline

**I1. [NEW] Keep PR scope minimal; defer unrelated refactors to follow-up PRs** (confidence: T1-accepted, PR #4202)

Reviewers asked authors to revert unrelated changes in several PRs and create follow-ups. AGENTS.md has no explicit guidance on PR scope.

Suggested AGENTS.md text (new subsection under Conventions):
```
### PR scope

- Keep each PR focused on a single logical change. If you encounter an unrelated bug
  or improvement while working on a PR, prefer one of:
  - Fixing it in a separate PR opened before or after this one.
  - Noting it in a `// TODO` comment with a follow-up issue.
  Avoid mixing refactors with behavioral changes in the same PR; reviewers have
  difficulty distinguishing intentional changes from side effects.
```

Evidence: PR #4202 — mkenigs: "Let's leave `render_legacy_exports` as is... we'll take that as followup"; acceptance_rate=1.0.

---

**I2. [EXPANSION] Clarify whether changes in a PR are related to the primary goal** (confidence: T1-accepted, PR #3869)

This complements I1. When a PR includes a fix that appears unrelated to the stated goal, explicitly state in the PR description or inline comment whether the fix is related or opportunistic.

Suggested AGENTS.md text:
```
  - When a PR touches code that is not central to the stated goal (e.g., fixing a
    pre-existing bug while refactoring), note in the PR description or an inline
    comment whether the change is related. This helps reviewers distinguish intentional
    scope from opportunistic fixes.
```

Evidence: PR #3869 — mkenigs: "question nonblocking: is this an unrelated bug fix?"; acceptance_rate=1.0.

---

### Group J: Async and Threading Discipline

**J1. [NEW] Filter filesystem watcher events early to prevent redundant state reads** (confidence: T1-accepted, PR #3968)

When using `notify` or similar filesystem watchers, filter events to write events only before reacting. Do not spin on every file system event.

Suggested AGENTS.md text:
```
  - **Filesystem watchers:** When watching a file for changes with `notify`, filter
    events immediately inside the watcher callback to `Create(File|Any)` and
    `Modify(Data(_)|Any)` events only. Reacting to Close, Access, or Remove events
    causes redundant reads and can produce busy loops.
```

Evidence: PR #3968 — dcarley: "We shouldn't spin on state changes every time the file is read"

---

**J2. [NEW] Add timeouts to blocking wait operations** (confidence: T1-raised, PR #3794)

When a function blocks indefinitely waiting for a signal (e.g., SIGUSR1 from a child process), consider whether a timeout is warranted to avoid hangs if the child process gets stuck.

Suggested AGENTS.md text:
```
  - **Blocking waits:** When blocking indefinitely on an inter-process signal or
    resource, document whether a timeout was considered and why it was or was not
    added. Add a timeout if the process being waited on could plausibly hang
    (e.g., due to resource exhaustion or a bug).
```

Evidence: PR #3794 — dcarley: "Should we ever timeout here in case the executive gets stuck?"; acceptance_rate=null.

---

**J3. [NEW] Document when to use async sandwich vs coloring functions as async** (confidence: T1-raised, PR #4122)

When a sync function needs to call async code (e.g., reqwest), document in a comment why the async-sandwich pattern was preferred over making the function async.

Suggested AGENTS.md text:
```
  - **Async sandwich pattern:** When a synchronous function must call async code,
    document why you chose the async-sandwich approach (wrapping in a blocking
    executor) rather than making the function `async`. This choice has implications
    for callers. If the function is only called from async contexts, prefer coloring
    it `async` directly.
```

Evidence: PR #4122 — mkenigs: "I don't love the sandwich pattern, but I think it makes it more confusing to have another async pattern. When should I follow the sandwich pattern, and when should I not?"; acceptance_rate=null.

---

### Group K: Manifest and TOML Editing

**K1. [EXPANSION] Preserve formatting context when patching TOML arrays in-place** (confidence: T1-accepted, PR #4106)

AGENTS.md already says "never serialize manifests by hand." This adds detail for the raw TOML editing case: when modifying an array element in-place using `toml_edit`, copy the surrounding `decor` (whitespace and comments) from the element being replaced, not from an element being added elsewhere.

Suggested AGENTS.md text (add under Manifest usage):
```
  - **TOML array in-place editing:** When replacing an element in a `toml_edit::Array`,
    copy the existing element's `decor` (whitespace and leading comments) to the
    replacement value. Do not copy decor from a neighboring element; this duplicates
    comments. Use `new_val.decor_mut().clone_from(old_val.decor())`.
```

Evidence: PR #4106 — mkenigs: "suggestion blocking: don't copy comments"

---

### Group L: Nix/Flake Configuration

**L1. [NEW] Add explanatory comments when using manual symlinking in Nix derivations** (confidence: T1-raised, PR #3960)

In `flake.nix` or package definitions, when manually creating symlinks (rather than using `makeWrapper` or similar), add a comment explaining why manual symlinking is required.

Suggested AGENTS.md text:
```
  - **Manual symlinking in Nix:** When using `ln -s` manually in a derivation rather
    than a higher-level helper (e.g., `makeWrapper`), add a comment explaining why the
    manual approach is needed and what would break if replaced with the helper.
```

Evidence: PR #3960 — mkenigs: "Slight preference to add a comment if you stick with manual symlinking"; acceptance_rate=0.0.

---

## Section 2: Reviewer Voice Notes

### ysndr (Tier 1)

ysndr's comments in this dataset cluster around two concerns: **architecture and lifecycle correctness**, and **design documentation**. When reviewing authentication code (PRs #3921, #4172), ysndr pushed back on sentinel values and missing variants, preferring that code carry richer semantic signal even in degraded states (an expired token is more informative than `""`; a `Credential::NoToken` variant may be less clear than `Option<Credential>`). In activation code (PR #3600), ysndr's review introduced the async `select!`-based signal handling pattern, reflecting a preference for explicit shutdown sequences that correctly release resources. In provider code (PR #3785), ysndr flagged removed doc comments as a loss — documentation is a first-class deliverable, not a by-product of implementation. The throughline is: **write code that carries its own context**, whether through richer types, richer comments, or richer error states.

### mkenigs (Tier 1)

mkenigs's comments throughout this dataset reflect concern for **reviewer and future-author usability**. Several comments ask whether a change is related to the PR's stated goal (PR #3869, PR #4202) — a signal that mkenigs expects PRs to be narrowly scoped and that surprises in the diff require explanation. mkenigs also raised testing coverage repeatedly (PRs #3715, #3869, #3968), asking for tests when bug fixes landed without them. In user-facing output, mkenigs flagged counterintuitive terminology (PR #3750) and unclear man page structure (PR #3651). In Cargo management (PR #3939), mkenigs noted workspace version drift from an uncoordinated inline version. The throughline is: **make every change easy for the next reviewer to evaluate** — tight scope, documented intent, tests where warranted, and consistent infrastructure conventions.

### dcarley (Tier 1)

dcarley's comments in this dataset reflect **operational and resource-pressure awareness**. When reviewing the filesystem watcher (PR #3968), dcarley flagged unnecessary spinning on every file event as a resource concern. When reviewing blocking waits (PR #3794), dcarley raised the question of timeouts for stuck processes. When reviewing breaking changes (PR #3803), dcarley asked whether the disruption could be framed as a user benefit. In code style, dcarley flagged double negatives in shell scripts (PR #3932), labelled blocks (PR #4045), and long lines that would fail the linter (PR #4093). The throughline is: **think about what happens in production at scale** — resource pressure, broken flows, and user-visible disruption are first-order concerns, not afterthoughts.

---

## Section 3: Recommended New AGENTS.md Sections

### Recommended: Add a "Testing conventions" subsection under Conventions

Currently AGENTS.md contains only `just test-all` style command documentation and a note about `assert_eq!` on structs. The gap-report findings suggest reviewers consistently expect:

1. Bug fixes to include unit tests.
2. TTY-dependent changes to document manual test steps.
3. Unit tests to be kept focused rather than serving as integration tests for side-heavy functions.

Proposed location: under `## Conventions`, add `### Testing conventions` before the existing `- **Test naming:**` bullet. Consolidate the `- **Test naming:**` bullet into this subsection.

Suggested text:
```markdown
### Testing conventions

- **Bug fixes require tests:** Every bug fix should include a unit test that
  reproduces the defect. If adding a test is impractical, note the gap in the
  PR description.
- **Focused unit tests:** Unit tests should test one function or concept.
  If a function is hard to unit-test due to side-effects, treat this as a
  design signal: consider extracting pure logic first.
- **TTY-dependent behavior:** When output behavior depends on terminal detection
  (color, pager, interactive prompts), document manual test steps in the PR
  description; automated tests cannot easily simulate tty contexts.
- **Test naming:** Do not prefix test functions with `test_`. [existing text]
- **Assert on entire structs:** [existing text]
```

---

### Recommended: Add a "PR scope" subsection under Conventions

No existing AGENTS.md guidance covers PR discipline. Multiple reviewers asked authors to defer unrelated changes to follow-up PRs. This is appropriate to codify.

Proposed location: under `## Conventions`, add `### PR scope` as a new subsection.

Suggested text:
```markdown
### PR scope

- Keep each PR focused on a single logical change. If you encounter an unrelated
  bug or improvement while implementing a PR, prefer one of:
  - Opening a separate PR for the fix.
  - Adding a `// TODO(<issue>):` comment and a follow-up issue.
- When a PR includes a change that appears unrelated to its stated goal, explain
  in the PR description whether the change is related or opportunistic. Reviewers
  cannot distinguish intentional scope expansion from accidental changes without
  this context.
- When deferring an improvement to a follow-up, record the intent in a tracking
  issue and reference it as `// TODO(<issue>): <description>` in the code.
```

---

## Section 4: Summary Table

| # | Amendment | Group | Tier | PR Evidence | Acceptance | Recommended |
|---|-----------|-------|------|-------------|------------|-------------|
| A1 | Use named constants (no magic numbers) | Code clarity | T1-accepted | #3801 | 1.0 | Yes |
| A2 | Place comments adjacent to code | Code clarity | T1-accepted | #3770 | 1.0 | Yes |
| A3 | Avoid labelled blocks | Code clarity | T1-raised | #4045 | null | Needs evidence |
| A4 | Avoid unnecessary clones | Code clarity | T2-only | #4172 | 1.0 | Weak |
| B1 | Singular enum variants | Naming | T1-accepted | #3870 | 1.0 | Yes |
| B2 | Distinguish config vs runtime type names | Naming | T2-only | #4172 | 1.0 | Weak |
| C1 | Use workspace deps in Cargo.toml | Dependencies | T1-accepted | #3939 | 1.0 | Yes |
| D1 | Send expired tokens for identification | Error handling | T1-accepted | #3921 | 1.0 | Yes |
| D2 | Diagnostic for unsupported build features | Error handling | T1-accepted | #4172 | 1.0 | Yes |
| E1 | Mark unstable JSON output in man pages | Docs | T1-raised | #3651 | 0.0 | Needs evidence |
| E2 | Mutually exclusive notation in man SYNOPSIS | Docs | T1-raised | #3651 | 0.0 | Needs evidence |
| E3 | Man page or TODO for hidden subcommands | Docs | T1-accepted | #3969 | 1.0 | Yes |
| E4 | Precise Nix terminology (targets vs artifacts) | Messaging | T1-accepted | #4232 | 1.0 | Yes |
| E5 | Frame breaking changes as benefits | PR process | T1-raised | #3803 | 1.0 | Needs evidence |
| F1 | Document edge cases with evidence inline | Comments | T1-accepted | #4215 | 1.0 | Yes |
| F2 | Deferred work: use TODO(<issue>) | Comments | T1-accepted | #3801, #4172 | 1.0 | Yes |
| F3 | Preserve doc comments during refactors | Comments | T1-accepted | #3785 | 1.0 | Yes |
| F4 | Document known races and accepted limits | Comments | T1-accepted | #3920 | 1.0 | Yes |
| F5 | Upstream workarounds: cite issue URL | Comments | T1-accepted | #3988 | 1.0 | Yes |
| G1 | Avoid double negatives in shell scripts | Shell | T1-accepted | #3932 | 1.0 | Yes |
| H1 | Tests for bug fixes and cross-component | Testing | T1-raised | #3869 | 0.0 | Needs evidence |
| H2 | Document manual testing for TTY behavior | Testing | T1-raised | #3672 | null | Needs evidence |
| H3 | Testability as design signal | Testing | T1-raised | #3903 | 0.0 | Needs evidence |
| I1 | PR scope: defer unrelated refactors | PR scope | T1-accepted | #4202 | 1.0 | Yes (new section) |
| I2 | Explain whether changes are related | PR scope | T1-accepted | #3869 | 1.0 | Yes (new section) |
| J1 | Filter filesystem watcher events early | Async | T1-accepted | #3968 | 1.0 | Yes |
| J2 | Add timeouts to blocking waits | Async | T1-raised | #3794 | null | Needs evidence |
| J3 | Document async sandwich vs coloring | Async | T1-raised | #4122 | null | Needs evidence |
| K1 | Preserve TOML array decor when patching | Manifest | T1-accepted | #4106 | 1.0 | Yes |
| L1 | Comment manual symlinking in Nix | Nix | T1-raised | #3960 | 0.0 | Needs evidence |

---

## Findings Declined and Why

The following findings from the 67-item dataset were **not proposed as AGENTS.md amendments**. Reasons are given for each.

| Finding ID | Rule statement (abbreviated) | Reason declined |
|------------|------------------------------|-----------------|
| 1015 | Use `select!` for signal handler + CLI completion | Too implementation-specific; describes one PR's architectural pattern for the `FloxArgs::run` method. Not a general rule. |
| 1051 | Clarify "environment's build context" in docs | Reviewer question ("I don't know what this means"), not a rule. The fix is to rewrite the specific phrase, not add a general guideline. |
| 1133 | Preserve force-flag behavior when branches are ahead | Describes a specific `flox pull --force` semantic decision. Not a generalizable rule. |
| 1186 | Align CLI and flox-activations verbosity behavior | Implementation-specific discussion about verbosity level mapping. Not a generalizable rule. |
| 1250 | Consider timeouts for blocking operations | Superseded by J2 (which captures the more actionable framing). The raw finding is a non-blocking observation without an accepted fix. |
| 1255 | Document async vs sync signal handling tradeoffs | Too implementation-specific to a single monitoring loop design. Subsumed by J3. |
| 1266 | Document intent behind fish hook output formatting | The comment explains a specific direnv compatibility decision. Not a rule; it is a one-time documentation need. |
| 1282 | Ensure logging covers all subsystems | Specific to `flox_activations` + `flox_watchdog` verbosity coupling. Not generalizable without more evidence. |
| 1286 | Hierarchical deduplication for dotted notation | Describes a specific design decision for invocation source tags. Not a general code rule. |
| 1290 | Document upstream issues in comments | Superseded by F5. The raw finding text is narrower. |
| 1302 | Preserve docs when refactoring | Superseded by F3. The raw finding is a weaker framing. |
| 1313 | Document downstream semantic purpose of design choices | Very general; too vague to be actionable. Subsumed by F1/F4. |
| 1328 | Consider async futures for parallel nix invocations | A specific architectural concern about thread count and Nix daemon connections. Not a general rule; flagged as a concern, not a finding. |
| 1332 | Rename `git_auth` to clarify Kerberos vs Auth0 | Single naming suggestion for a specific module. Not a general naming rule. |
| 1342 | version and outputs cannot be used together | Describes a resolved design constraint. Not a rule. |
| 1344 | Extract flag logic into named enum | Implementation-specific refactor suggestion. The extracted `AutoSetupBehavior` enum is already in the codebase. |
| 1349 | Avoid labelled blocks | Superseded by A3 (which is the clearer framing from PR #4045). |
| 1352 | Handle all error cases; don't panic in library code | Panic discipline is already partially covered. The raw finding describes a specific question about cleanup semantics, not a new general rule. |
| 1357 | Use expired tokens for identification | Superseded by D1. |
| 1368 | Use `Option<T>` to distinguish absence from presence | Superseded by B2 (config vs runtime type naming). The raw finding overlaps but is narrower. |
| 1373 | Document race conditions | Superseded by F4. |
| 1375 | Document when ephemeral activation is preferred | Describes a specific design question in services/restart. Not a general rule. |
| 1386 | Comment manual symlinking in Nix | Captured by L1. |
| 1396 | Document async sandwich vs coloring | Captured by J3. |
| 1398 | Record metrics for catalog build operations | Product-specific telemetry suggestion. Not an AGENTS.md rule. |
| 1403 | Point users to signing key documentation | Error message content suggestion for a specific error variant. Not a general rule. |
| 1419 | Prefer deterministic merge behavior to max-version logic | Manifest-specific design trade-off discussion. Not a general rule. |
| 1427 | Did you mean to remove all comments? | Single-question reviewer comment. Not a rule. |
| 1456 | Break long chains to satisfy line length | rustfmt enforces this. Not a rule for AGENTS.md. |
| 1463 | Use `peekable()` instead of collecting to Vec | Performance micro-optimization suggestion. Not a general rule; acceptance_rate=0.0. |
| 1000 | Rate-limit upgrade notification fetches | Specific optimization suggestion for `check_for_upgrades`; acceptance_rate=0.0 (suggestion not adopted). |
| 1002 | Rate-limit or cache expensive operations | Broader version of #1000; acceptance_rate=0.0. |
| 1021 | Avoid unnecessary clones | Superseded by A4. |
| 1035 | Test coverage for generation switching | Superseded by H1. |
| 1036 | Unit test coverage for bug fixes | Superseded by H1. |
| 1045 | Warn on upgrade with force pushes / pinned envs | Specific unimplemented product behavior. Not a coding rule. |
| 1053 | Mark unstable API outputs | Superseded by E1. |
| 1057 | Flag counterintuitive terminology for review | Reviewer meta-comment, not a rule. |
| 1059 | Link Nix expression builds to documentation | Specific to a new man page. Not a general rule. |
| 1066 | Frame breaking changes as benefits | Superseded by E5. |
| 1069 | Man page or TODO for feature-flagged subcommands | Superseded by E3. |
| 1084 | Precise terminology: targets vs artifacts | Superseded by E4. |
| 1101 | Document proptest field-count constraints | Niche testing-framework-specific guidance. Needs more evidence. |
| 1107 | Sync test data files with JWT token claims | Test fixture maintenance note. Not a general rule. |
| 1111 | Add comprehensive tests for edge cases | Too vague. Subsumed by H1. |
| 1130 | Clarify lock scope in build operations | Reviewer question about a specific lock; not a rule. |
| 1149 | Mutually exclusive option notation in man pages | Superseded by E2. |
| 1152 | Extract larger subsystems into dedicated modules | Reasonable general refactoring guidance, but T2-only and too vague without more context. |
| 1161 | Distinguish AuthnMode from AuthContext | Superseded by B2. |
| 1164 | Document manual testing for TTY behavior | Superseded by H2. |
| 1176 | Diagnostic for unsupported auth modes | Superseded by D2. |
| 1209 | Use named constants (STDERR_FILENO) | Superseded by A1. |
| 1216 | Avoid double negatives in shell | Superseded by G1. |
| 1222 | Set env var defaults across CLI versions | Specific to a compatibility regression with `_FLOX_ENV_CUDA_DETECTION`; acceptance_rate=0.0. Not a general rule. |
| 1234 | Test cleanup_pid as a no-op | Specific test-case suggestion for a single function. Not a general rule. |
| 1241 | Preserve temp files for debugging | Reviewer asked for this; acceptance_rate=0.0 (not adopted). Specific to startup scripts. |
| 1244 | Place comments adjacent to code | Superseded by A2. |
| 1250 | Timeout for blocking operations | Superseded by J2. |
| 1262 | Minimize refactoring scope in PRs | Superseded by I1. |
| 1329 | Singular enum variant names | Superseded by B1. |

---

*Report generated from 67 findings (in_agents_md=0, confidence_score≥0.5) extracted from 944 review comments across 216 PRs. All findings have total_evidence_count=1.*
