# Conventions for `cli/flox-rust-sdk/src/models/environment/`

This directory contains the core environment model implementations (`ManagedEnvironment`, `PathEnvironment`, `RemoteEnvironment`, `CoreEnvironment`) and their supporting modules (`floxmeta_branch`, `generations`, `install`, `uninstall`, `fetcher`). Reviewers most often catch: missing test coverage for new error paths; error chains being flattened to strings instead of boxed; raw manifest strings smuggled past the typed `Manifest<S>` boundary; auth-result errors silently swallowed via `.ok()`; and build-skipping or locking logic applied in one method but not in a parallel one that does the same operation.

## Area-specific rules

### Testing

**Always add a unit or integration test for every new error path, including newly introduced variants such as `ManagedEnvironmentError::Diverged(DivergedMetadata)`.**
New code paths without tests are routinely flagged as blocking. If a bats test already exists, add an assertion on the specific output rather than relying on the test passing silently.
Evidence: PR #3646, PR #4076.

**When adding assertions to integration tests, assert against the generated summary message content, not just exit codes.**
Reviewers have pointed out cases where a bats test passes but says nothing about what the error message contains; a follow-up assertion against generation summary strings (e.g., local vs. remote generation descriptions) is expected.
Evidence: PR #3646.

**Use `assert_eq!` on the entire struct, not on individual fields, and do not drop output assertions when consolidating tests.**
When collapsing multiple tests into one, check that every prior assertion is still present. A consolidation that silently removes the "Next steps" tip assertion is a coverage gap that reviewers will call out.
Evidence: PR #4114.

**Add unit tests for new helper functions extracted into their own files (e.g., `install.rs`, `uninstall.rs`) covering both the happy path and error paths.**
Reviewers treating a new module without tests as a blocker is the established pattern in this area.
Evidence: PR #4076.

---

### Error variant design

**When multiple source error types must map to a single enum variant, wrap with `Box<dyn std::error::Error + Send + Sync>` or `io::Error::new(ErrorKind::Other, e)` — never convert to a display string via `display_chain()`.**
Converting to a string destroys the error chain; downstream handlers and logging lose the ability to inspect the original cause. Prefer separate variants when they are semantically distinct.
Evidence: PR #3673.

**Fix typos in `#[error(...)]` strings immediately, and update all tests that assert on the exact text of that error.**
A typo in an error variant that appears in bats test output will break CI. The fix and the test update belong in the same commit.
Evidence: PR #3646.

**Distinguish `CloneBranch` from `FetchBranch` error variants when routing git remote errors to typed environment errors.**
These two variants represent different operations; conflating them misroutes access-denied and ref-not-found errors. Match on the correct variant at the relevant call site.
Evidence: PR #3717.

**Do not silently discard authentication errors by calling `.ok()` on the result of `get_handle()` or similar auth accessors.**
Using `.ok()` hides whether the user is not logged in or whether there was a network/communication error. If the `None` case is intentional (no user hint available), document why in a comment.
Evidence: PR #4047.

**Use the return type (`Option` vs. `Result`) to encode authentication semantics explicitly.**
`Option<String>` for a handle that may simply not exist is correct; `Result` is required when failure indicates a real problem (missing login, network error). Choosing the wrong type forces callers to discard error information.
Evidence: PR #4047.

---

### Manifest lifecycle in environment models

**Never store or pass raw manifest strings; use `Manifest<S>` typed constructors for every manifest read and write.**
Returning `Option<String>` from `UninstallationAttempt` (instead of `Option<Manifest<Migrated>>`) re-introduces the untyped manifest anti-pattern that the type-state design eliminates. Use the typed constructors defined in the `flox-manifest` crate.
Evidence: PR #4076.

---

### Build-skipping and locking consistency

**When adding a new method that calls `build()` on a `CoreEnvironment`, check `rendered_env_links` (and any analogous method) for build-skipping logic and replicate it.**
The existing `rendered_env_links` method contains caching/skipping logic (introduced in PR #2705) to avoid unnecessary Nix builds. A new method such as `rendered_env_links_for_generation` that bypasses this logic can trigger expensive redundant builds.
Evidence: PR #3638.

**Acquire the floxmeta lock at every entry point that may trigger a git fetch, not only at the outermost open call.**
Git writes an `index.lock` during fetch; if a second code path (e.g., `ensure_generation_lock`) fetches without holding the floxmeta lock, concurrent operations will collide. Add lock acquisition at any boundary that issues a network-touching git operation.
Evidence: PR #3717.

**Document the intended scope and lifetime of each lock: must it be held through the entire build, or just through the git fetch?**
Reviewers have flagged lock variables that are acquired but whose intended scope is unclear. A short comment (`// held through build to prevent concurrent fetch`) saves future confusion.
Evidence: PR #3717.

---

### Semantic correctness and control flow

**After any logic change to a branch condition, verify that all arms of the surrounding `if`/`match` remain reachable.**
Introducing a new intermediate variable (e.g., `let is_uptodate = ...`) can silently make a branch dead. Run the affected tests and check with clippy before merging.
Evidence: PR #3869.

**Preserve the behavior of `--force` flags carefully: `flox pull --force` must reset local state to upstream even when the local branch is ahead.**
The interaction between `checkout_valid`, `is_uptodate`, and `force` is subtle. A change that appears to simplify the condition can break the case where the user is ahead of FloxHub but wants to reset to it.
Evidence: PR #3869.

**Verify output message formatting makes semantic sense before changing a `Display` implementation or format string.**
A change from `format!("{id} ({url})")` to `format!("{id} ({pkg})")` (delegating to `pkg`'s `Display`) must produce output that is meaningful to the user; reviewers will check what the rendered string looks like.
Evidence: PR #4075.

---

### Documentation and comments

**Add a doc comment to every non-obvious struct in this area — especially `FloxmetaBranch`, `Generations`, and `GenerationLock` — explaining what the type represents, its invariants, and how it relates to adjacent types.**
These structs sit at the intersection of git, Nix, and environment lifecycle concerns; without a doc comment a reviewer has to reconstruct their semantics from the implementation. A one-paragraph description is sufficient.
Evidence: PR #3813.

**When a code path contains a deliberate shortcut for an edge case (e.g., `outputsToInstall = None`), document its practical rarity with concrete evidence (e.g., the nixpkgs `stdenv.mkDerivation` behavior) rather than merely noting the type is `None`.**
A comment that says "this can be None" adds nothing beyond the type signature. A comment explaining that nixpkgs `stdenv` always populates `outputsToInstall` via `commonMeta`, so `None` only occurs in non-stdenv derivations or catalog bugs, gives reviewers the context to decide whether the shortcut is acceptable.
Evidence: PR #4215.

**Comments on `Option` fields must enumerate all semantically distinct `None` cases, including future ones.**
If `None` currently means "currently live generation" but a future state could introduce "never been live", note both in the doc comment. A comment that only documents the current use case will silently mislead future engineers.
Evidence: PR #3652.

---

### Code organization

**Drop `pub` visibility on module-internal helper functions and order functions in dependency sequence (callers before callees, or by logical flow).**
Functions that are implementation details of a single public entry point — such as a helper used only by `resolve_specs_to_modifications` — should not be `pub`. Ordering them after the public function they serve makes the module easier to read top-down.
Evidence: PR #4076.

---

### Unrelated changes

**If a PR includes an unrelated bug fix, make it explicit in the PR description and consider extracting it into a separate commit.**
Reviewers will notice and ask. An unrelated fix buried in a feature PR can also break tests in unexpected ways if the two changes interact.
Evidence: PR #3869.

## Cross-cutting reminders

- Error type hierarchy (extending variants vs. string-matching at call sites) — `.claude/skills/flox-rust-review/SKILL.md`
- Provider trait and associated type design — `.claude/skills/flox-rust-review/SKILL.md`
- Manifest type-state lifecycle (`Manifest<S>` constructors, `ManifestLatest`) — `.claude/skills/flox-rust-stylistic-conventions/SKILL.md`
- User-facing message syntax, sentence structure, and emoji conventions — `.claude/skills/flox-rust-stylistic-conventions/SKILL.md`
- Test naming (no `test_` prefix, descriptive names) — `.claude/skills/flox-rust-stylistic-conventions/SKILL.md`
- `use` statement placement (module scope, not function scope) — `.claude/skills/flox-rust-stylistic-conventions/SKILL.md`

## When in doubt

Most active reviewers in this area: **ysndr** (T1), **dcarley** (T1) — the two with the highest comment counts. mkenigs (T1) is also active. Their prior comments are the best precedent — see PRs #3638, #3646, #3652, #3673, #3717, #3813, #3869, #4045, #4047, #4075, #4076, #4114, #4215.
