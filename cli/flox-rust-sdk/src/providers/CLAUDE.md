# Conventions for `cli/flox-rust-sdk/src/providers/`

Code review in the providers area consistently flags four themes: error-type hierarchy discipline (add typed enum variants rather than string-match at call sites), auth-flow correctness (Kerberos and Auth0 branches must each preserve pre-refactor behavior and be traced), deprecation hygiene (stale `MockClient` dispatch arms and competing `From` implementations must be actively removed rather than accumulated), and provider-trait design (prefer concrete types over unnecessary generic bounds or associated-type constraints). The `publish.rs` file receives the most attention and concentrates most of the rules below; `catalog.rs`, `auth.rs`, and `nix.rs` are the other high-traffic files.

## Area-specific rules

### Error classification at the provider boundary

**Add typed enum variants for new failure modes; never match on `.to_string()` output at call sites.**
The git provider uses `GitCommandError` → `GitRemoteCommandError` with typed variants (`AccessDenied`, `Diverged`, `RefNotFound`). When you need to classify a new failure, extend the enum at the provider boundary — not by string-matching inside `gather_build_repo_meta` or any downstream consumer.
Evidence: PR #4165, PR #4154

**Extend `GitCommandGetOriginError` with a `Remote(GitRemoteCommandError)` variant instead of re-implementing access-denied detection at call sites.**
Keeping classification inside the provider means all callers automatically benefit from better error information. Checking `is_access_denied(&msg)` in `publish.rs` is a workaround that signals a missing variant.
Evidence: PR #4165

**Do not convert one typed error variant into a semantically different one to silence it.**
Converting `CreateNetrc` into `NoToken` to avoid an early-return is a semantic lie — the real failure is netrc creation, not a missing token. Defer netrc creation to the point it is actually required so the error can be surfaced accurately.
Evidence: PR #4154

**Reserve `panic!` for programmer-error invariants; use typed variants or `Custom(Box<dyn Error + Send + Sync>)` for expected-but-unhandled operational errors.**
Unexpected mutex poisoning or unrecoverable setup failures are acceptable `panic!` sites. File I/O failures and external-process errors that users may encounter must be surfaced as typed error variants or the `Custom` catch-all.
Evidence: PR #3785

**Trim stderr output before embedding it in error messages.**
Trailing newlines from git or nix stderr will corrupt formatted messages. Apply `.trim()` before interpolating into a message string.
Evidence: PR #4096

**Treat expected I/O failures as first-class error variants, not `.unwrap()` or `expect()`.**
File I/O inside providers (reading netrc, checking git ls-files output, etc.) can fail for legitimate operational reasons. Model them as typed variants so callers can distinguish them.
Evidence: PR #3785

### Auth-flow correctness and tracing

**Preserve pre-refactor behavior for each `AuthContext` branch when restructuring auth provider construction.**
Kerberos mode must construct `NixAuth { floxhub_token: None, ... }` because `create_netrc()` handles missing tokens gracefully (`NoToken` / `None` returns). Returning an error from the Kerberos arm is a regression that breaks all auth'ed nix operations for Kerberos users.
Evidence: PR #4172

**Add a `tracing::debug!` call at each auth-flow branch so the chosen mode is visible in traces.**
A log line like `"Kerberos mode — git auth handled natively via ccache"` is the minimum. Each `AuthContext` arm in `apply_git_auth` / `authenticate` should have a distinct log statement so traces are self-explanatory without reading source.
Evidence: PR #4172

**Extract git-auth application logic as an extension trait (`GitCommandOptionsExt`) rather than a free function.**
An extension trait keeps authentication per-variant logic close to the type it operates on and makes the three distinct behaviors (bearer, Kerberos no-op, empty credential helper) discoverable at the trait call site.
Evidence: PR #4172

### Deprecated infrastructure: active removal

**When a new `MockClient` method is needed but the mock dispatch pattern is being deprecated, use `unimplemented!()` and remove any `Response::*` enum arm.**
Do not add new `Response::CheckBuild` dispatch logic to `MockClient`. Use `unimplemented!("... not supported in MockClient")` and also delete any unit tests that were exercising the deprecated mock arm.
Evidence: PR #4156

**Delete deprecated `From` implementations when adding schema-version-specific converters.**
Adding a new `impl From<ServicesV1> for ProcessComposeConfig` while leaving the old `impl From<Services>` creates competing converters. Remove the deprecated one in the same PR.
Evidence: PR #4152

### Provider trait design

**When only one implementation exists and mocks are not realistic, take the concrete type directly.**
`gather_build_repo_meta(git: &impl GitProvider<GetOriginError = GitCommandGetOriginError>)` is a pinned-associated-type constraint that hides the fact that only `GitCommandProvider` is ever used. Take `&GitCommandProvider` directly instead.
Evidence: PR #4165

**Move helper logic to provider trait methods to ensure consistency across call sites.**
If `check_env_files_tracked` runs git commands that make sense as part of the git provider's contract, move them to the provider rather than reimplementing raw subprocess calls in `publish.rs`.
Evidence: PR #4102

**Use singular form for enum variant names** (e.g., `AuthStrategies::Auth0`, not `AuthStrategies::Auth0s`).
The codebase uses singular names throughout (`AuthStrategy::Auth0`, `AuthStrategy::Kerberos`). Follow this convention for new auth-related enums.
Evidence: PR #3870

### Manifest constructor discipline in providers

**Use `lockfile.migrated_manifest()` / `lockfile.migrated_user_manifest()` — never call `lockfile.manifest.migrate_typed_only(Some(&lockfile))` directly.**
The helper methods encapsulate the migration logic. Calling the inner method directly at call sites bypasses that encapsulation and will require manual updates when the API changes.
Evidence: PR #4161

### User-facing messages at the provider boundary

**Show the joined path in error messages, not path components separately.**
Write `{expression_dir}/pkgs` rather than `{expression_dir}` + `"pkgs"` so users do not have to mentally concatenate path segments.
Evidence: PR #4096

**Use relative paths (relative to the user's project) in error messages, not absolute paths from internal clean-checkout directories.**
Users do not know about temporary clean-checkout paths. Show paths relative to the working tree root the user is actually in.
Evidence: PR #4102

**When an error references a missing upstream, name the actual remote.**
`"Current branch 'main' has no upstream remote configured. Set one with 'git branch --set-upstream-to=<remote>/main'"` is more actionable when `<remote>` is replaced by the actual remote name discovered at runtime (e.g., via `git.remotes()`).
Evidence: PR #4165

**Verify what condition actually triggers an error message before changing it.**
Branch names and remote branches may diverge — what is required is that the rev exists on the remote branch, not that the branch names match. Describe the actual invariant being checked, not an approximation.
Evidence: PR #4165

**Point catalog signing-key errors at documentation covering both default (Flox-installer) and custom catalog-store setups.**
An error like `BuildPublishedPackage` or `UntrustedPublicKey` that appears during custom catalog store usage should link to docs that address both cases, since users may be in either situation.
Evidence: PR #3992

### Code organization and style

**Use `formatdoc!` for all multi-line formatted strings; avoid `format!` with backslash line continuations.**
`formatdoc!` from the `indoc` crate handles indentation correctly and makes strings readable in source. The backslash continuation form is fragile and harder to read.
Evidence: PR #4165

**Consolidate parallel code paths into a single helper when they are semantically equivalent.**
`get_build_output_nar_infos_local` duplicated `get_build_output_nar_infos` when calling with `"daemon"` as the store URL would have been sufficient. Prefer a single function with a parameter over parallel copies.
Evidence: PR #4140

**Use explicit auth-method constants in tests — not `Default::default()` — to make test intent unambiguous.**
`Default::default()` for `AuthMethod` may change meaning as new auth modes are added. Tests that require a specific auth mode should name it: `AuthMethod::Auth0`.
Evidence: PR #4047

**Use table-driven tests when many small cases share the same fixture boilerplate.**
Evidence: PR #3772

### Semantic correctness checks before merging

**Verify field relationships match the domain model before merging.**
`dot_flox_dir` should be the `.flox` directory itself, not its parent. Double-check ownership semantics and directory relationships when mapping environment metadata to API request payloads.
Evidence: PR #4096

**Confirm which context an error path assumes (custom catalog vs. custom catalog store).**
The distinction matters for error messages and for which documentation to point users toward. Verify the assumption is correct for all paths that reach the error.
Evidence: PR #3992

**Coordinate header-format migrations with the catalog server team.**
When changing from a boolean header (`flox-ci: true`) to a structured multi-value header, confirm whether the server needs to support both formats during a transition window. Document the migration plan in a code comment.
Evidence: PR #3939

## Cross-cutting reminders

- Error-type hierarchy, typed variants, and `Display`-impl sanitization — `AGENTS.md` "Error handling architecture" section
- Provider trait vs. concrete type tradeoffs, associated-type constraints — `AGENTS.md` "Provider traits and associated types" section
- Manifest constructor discipline (`Manifest::read_typed`, `migrated_manifest()`) — `AGENTS.md` "Manifest usage" section
- User-visible message structure, sentence case, no-dead-ends, emoji map — `AGENTS.md` "User-visible message syntax" section
- Deprecated infrastructure identification before adding implementations — `AGENTS.md` "Deprecated infrastructure" section

## When in doubt

Most active reviewers: **ysndr** (T1), **mkenigs** (T1), **dcarley** (T1). The provider layer has heavy AGENTS.md coverage on error-type hierarchy and trait-vs-concrete-type tradeoffs — re-read those sections when refactoring auth, git, or catalog providers. For publish-specific questions, `dcarley` and `ysndr` have reviewed nearly every PR touching `publish.rs`.
