# `flox hook` Auto-Activation Demo Script

This script walks through the new `flox hook` functionality, which automatically
activates Flox environments when you `cd` into a directory containing a `.flox`
environment — no `flox activate` needed.

---

## Prerequisites

Build the branch first:

```bash
nix develop -c just build
export PATH="$PWD/target/debug:$PATH"
```

---

## 1. Install the Shell Hook

The `flox hook` command outputs shell-specific code that integrates with your
prompt to watch for `.flox` environments as you navigate directories.

```bash
# See what the hook looks like (inspect the generated code)
flox hook zsh

# Install it into the current shell session
eval "$(flox hook zsh)"    # or bash, fish, tcsh
```

> **What it does:** Registers a `precmd`/`PROMPT_COMMAND` function that runs
> `flox hook-env` on every prompt. When you `cd` into a directory with a `.flox`
> environment, it detects it automatically.

---

## 2. Create a New Environment (Auto-Trusted)

```bash
# Create a fresh project directory
mkdir -p /tmp/demo-project && cd /tmp/demo-project

# Initialize a Flox environment — it's automatically trusted AND activated
flox init
```

> **Key point:** `flox init` now auto-trusts the environment it creates *and*
> the shell hook activates it immediately — no `cd` away and back needed.
> After `flox init`, the environment is already active in your shell.

---

## 3. Install a Package and See Auto-Activation

```bash
# Install a package into the environment
flox install hello

# Leave and return — watch it auto-activate!
cd ~
cd /tmp/demo-project

# The environment is now active — verify:
which hello
hello
```

> When you `cd` back into the project, the shell hook detects the `.flox`
> directory, checks that it's trusted, builds/locks the environment, and
> injects the environment variables (PATH, etc.) into your shell. No
> `flox activate` needed!

---

## 4. Trust & Untrusted Environments

Clone someone else's project to show the trust workflow:

```bash
# Simulate receiving an environment you didn't create
mkdir -p /tmp/untrusted-project/.flox/env
cat > /tmp/untrusted-project/.flox/env/manifest.toml << 'EOF'
version = 1

[install]
cowsay.pkg-path = "cowsay"
EOF

cd /tmp/untrusted-project
```

> **You'll see a message like:**
> ```
> flox: environment at '/tmp/untrusted-project/.flox' is not trusted.
> Run 'flox trust --path /tmp/untrusted-project/.flox' to auto-activate it.
> ```

```bash
# Trust the environment
flox trust

# Now cd away and back — it auto-activates!
cd ~
cd /tmp/untrusted-project
which cowsay
cowsay "Auto-activated!"
```

---

## 5. Deny an Environment

```bash
# You can also permanently deny an environment
cd /tmp/untrusted-project
flox trust --deny

# Now cd away and back — it stays inactive (no prompt, no activation)
cd ~
cd /tmp/untrusted-project
```

> Denied environments are silently skipped. Deny always takes priority over
> allow. You can re-trust later with `flox trust`.

---

## 6. Deactivate an Auto-Activated Environment

```bash
# Re-trust and cd back in to activate
cd /tmp/demo-project

# Verify it's active
which hello

# Deactivate without leaving the directory
flox deactivate

# Verify it's deactivated
which hello   # should no longer find it
```

> `flox deactivate` suppresses auto-activation for the current directory so
> the hook doesn't immediately re-activate the environment. The suppression
> lasts for the duration of the shell session.

---

## 7. Manifest Change Revokes Trust (Security)

```bash
cd /tmp/demo-project

# Verify auto-activation is working
which hello

# Now modify the manifest (simulating a git pull with changes)
flox install cowsay

# cd away and back
cd ~
cd /tmp/demo-project
```

> **Key security feature:** Trust is keyed on `blake3(path + manifest content)`.
> If the manifest changes (e.g., after a `git pull`), trust is revoked and you'll
> be prompted to re-trust. This prevents a malicious manifest change from
> silently executing in your shell.

---

## 8. Nested Environments

```bash
# Create a nested project inside the demo
mkdir -p /tmp/demo-project/subproject && cd /tmp/demo-project/subproject
flox init
flox install jq

cd ~
cd /tmp/demo-project/subproject

# Both the parent and child environments are active!
which hello   # from parent
which jq      # from child
```

> The hook walks the ancestor chain and discovers all `.flox` directories.
> All trusted environments are activated simultaneously.

---

## Cleanup

```bash
rm -rf /tmp/demo-project /tmp/untrusted-project
```

---

## Summary of New Commands

| Command | Description |
|---------|-------------|
| `flox hook <shell>` | Output shell hook code for auto-activation |
| `flox hook-env --shell <shell>` | Internal: called by the hook on each prompt |
| `flox trust` | Trust the environment in the current directory |
| `flox trust --deny` | Deny auto-activation for the current environment |
| `flox deactivate` | Deactivate an auto-activated environment |
