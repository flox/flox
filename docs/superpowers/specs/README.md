# Specs

Design specs for upcoming or in-flight work. Each spec is the source of truth for its
feature: implement it by running the `writing-plans` workflow on the spec, then executing
test-first.

| Spec | Status |
|------|--------|
| [2026-06-22-floxhub-token-storage-preference-design.md](2026-06-22-floxhub-token-storage-preference-design.md) | **Approved — ready to implement.** Makes `--insecure-storage` a persistent `floxhub_token_storage = "keyring" \| "plaintext"` config preference, with `--insecure-storage --once` for a temporary plain-text login. Follow-up to PR #4420 (`claude[bot]` review item #2). **Start here:** run `writing-plans` on the spec, then implement test-first. |
