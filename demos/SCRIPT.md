# `flox run` Phase 2 — Video Recording Script

A scene-by-scene script for recording the phase-2 (command-first)
behavior of `flox run`, as documented in `man flox-run`. Every command
below runs offline and deterministically against a bundled catalog
mock (`demos/demo-mocks.yaml`) that contains real, substitutable
store paths — the packages genuinely download and execute.

**What this demonstrates (SL-002):** command-to-package lookup without
`-p`, the exact-name-match silent default, the disambiguation prompt,
saved preferences, `--reselect`, explicit `--package` (which also
saves), non-interactive degradation, `@version` constraints, and
`flox search --command`.

---

## Setup (before recording)

From the repo root on the `feat/flox-run-binary-first` branch:

```bash
nix develop            # dev shell
just build             # builds ./target/debug/flox
just build-manpages    # for the man page scene
```

Then, in the terminal you will record:

```bash
export _FLOX_USE_CATALOG_MOCK="$PWD/demos/demo-mocks.yaml"
export FLOX_CONFIG_DIR="$(mktemp -d)"   # fresh config: no saved preferences
alias flox="$PWD/target/debug/flox"
clear
```

Notes:

- The mock was recorded on `aarch64-darwin`; record on an Apple
  Silicon Mac.
- Only the commands in this script are in the mock. Other
  packages/commands will fail with a lookup error — stay on script.
- **Retakes:** re-run `export FLOX_CONFIG_DIR="$(mktemp -d)"` to reset
  saved preferences to a clean slate.
- Packages were pre-substituted into the local Nix store when the mock
  was recorded, so runs are instant (no download spinners). Good for
  pacing.

---

## Scene 1 — The pitch (narration only)

> "Phase 1 of `flox run` shipped: you can run any package's command
> without installing it — but you had to name the package with `-p`.
> Phase 2 removes that: you just name the *command*, and Flox asks the
> FloxHub command-to-package index which package provides it."

No typing in this scene.

## Scene 2 — Just run a command

> "No package name, no environment, no cleanup."

```bash
flox run hello
```

**Expect:** `Hello, world!` — instantly, no prompt.

> "Behind the scenes, *two* packages provide a `hello` command. One of
> them is named exactly `hello`, and per the resolution rules the
> exact name match wins silently — no prompt."

## Scene 3 — The command is not the package

> "The whole point of the index: you don't need to know that `rg`
> lives in the `ripgrep` package."

```bash
flox run rg -- --version
```

**Expect:**

```text
Running 'rg' from package 'ripgrep'.
ripgrep 15.1.0
```

> "One package provides `rg`, so it runs silently — with a note about
> which package was used. (The `--` is only needed here so
> `--version` reaches `rg` rather than flox.)"

## Scene 4 — Inspect the index with `flox search --command`

```bash
flox search --command rg
flox search --command python3
```

**Expect:** `ripgrep` for the first; `python312` and `python313` for
the second.

> "The same index powers search. And `python3` is interesting —
> two packages provide it, and neither is named `python3`…"

## Scene 5 — Disambiguation prompt

> "When several packages provide the command and none matches its
> name exactly, Flox asks — once."

```bash
flox run python3 -c 'print("hello from flox run")'
```

**Expect:** an interactive menu:

```text
! Multiple packages provide 'python3'. Which would you like to use?
> python312 (python3) — High-level dynamically-typed programming language
  python313 (python3) — High-level dynamically-typed programming language
[Use arrow keys to select, Enter to confirm]
```

Press **Down**, then **Enter** to choose `python313`.

**Expect:**

```text
Saved 'python313' as the package for 'python3'. Use 'flox run --reselect python3' to change it.
hello from flox run
```

## Scene 6 — The choice is remembered

> "Run it again — no prompt. The choice was saved as a preference."

```bash
flox run python3 -c 'import sys; print(sys.version)'
```

**Expect:** Python 3.13.x version output, silently.

> "Preferences live in the regular Flox config:"

```bash
flox config | grep command_preferences
```

**Expect:** `command_preferences` containing `python3 = "python313"`.

## Scene 7 — Change your mind with `--reselect`

```bash
flox run --reselect python3 -c 'import sys; print(sys.version)'
```

Press **Enter** to choose `python312` this time.

**Expect:** the save message for `python312`, then Python 3.12.x
version output.

## Scene 8 — Explicit `--package` also saves

> "`--package` skips the lookup entirely — and per the latest spec it
> saves the mapping too, so the next bare run uses it."

```bash
flox run -p python313 python3 -c 'print("explicit")'
```

**Expect:**

```text
Saved 'python313' as the package for 'python3'. Use 'flox run --reselect python3' to change it.
explicit
```

## Scene 9 — Non-interactive: never prompts, never hangs

> "In a pipe or CI there is no terminal. With a saved preference the
> run is silent; without one, ambiguity fails fast with the candidates
> listed inline."

```bash
flox config --delete command_preferences.python3
echo '' | flox run python3
```

**Expect:**

```text
❌ ERROR: Multiple packages provide 'python3' and no preference is saved.
Packages with this command: python312, python313
Use 'flox run --package <PACKAGE> python3' to specify a package.
```

## Scene 10 — Version constraints on `--package`

```bash
flox run -p hello@2.12 hello
```

**Expect:** `Hello, world!`

> "Version constraints ride on the package spec — new in phase 2;
> phase 1 rejected `@`."

## Scene 11 — It's all in the man page

```bash
man ./build/flox-manpages/share/man/man1/flox-run.1
```

Scroll through **Package Selection**, **Saved Preferences**, and
**Non-Interactive Invocations**, then quit (`q`).

## Outro (narration)

> "To recap: name the command, not the package. Exact matches and
> single providers run silently; ambiguity prompts once and is
> remembered; `--reselect` and `--package` put you in control; and
> scripts never hang. Backed by an accurate command-to-package index
> on FloxHub — not a name heuristic — so `rg` finds ripgrep and
> `readelf` finds binutils."

---

## Appendix — Running against a live FloxHub instead of the mock

For a live-backend demo (e.g. architecture review), run the local
floxhub stack on the `feat/binary-to-package-index` branch (adds the
`packages/by-binary/{binary_name}` endpoint and the 2.4.5 migration),
point the CLI at it, and populate the `catalogs.package_binaries`
index — either by running `src/catalog-updater/src/collect-binaries.py`
against the binary cache, or by seeding rows directly (see the
fixtures in `src/catalog-server/tests/test_by_binary.py` for the
shape). Then drop `_FLOX_USE_CATALOG_MOCK` from the setup above.
Against a FloxHub *without* the endpoint, the CLI falls back to a
search-based exact-name heuristic, which finds `hello` but not
`rg`→`ripgrep`.
