# Specs

Design specs for upcoming or in-flight work. Each spec is the source of truth for its
feature: implement it by running the `writing-plans` workflow on the spec, then executing
test-first.

| Spec | Status |
|------|--------|
| [2026-06-22-floxhub-token-storage-preference-design.md](2026-06-22-floxhub-token-storage-preference-design.md) | **Implemented** in PR #4422. Makes `--insecure-storage` a persistent `floxhub_token_storage = "keyring" \| "plaintext"` config preference, with `--insecure-storage --once` for a temporary plain-text login. Follow-up to PR #4420 (`claude[bot]` review item #2). |
| [2026-06-22-implicit-reauth-token-storage-followup.md](2026-06-22-implicit-reauth-token-storage-followup.md) | **Deferred follow-up.** Implicit re-auth (`flox push`/`publish`/… on an expired token) does not honor a `plaintext` preference — an accepted P2 limitation from PR #4422. Documents the three fix options (thread, relocate-and-carry, accept) with blast radius and implications for when we pursue it. |
