# Porting the NixOS module (#2873) to generic systemd Linux

Worktree base: `d58979955` (the #2873 merge commit — the module is **not**
in `main` yet). Branch: `flox-systemd/generic-linux-module`.

Source under study: `modules/nixos/{default,common,services,overrides,pull,check}.nix`

## Goal

Give users on any systemd distro a turnkey-ish way to install a set of
systemd files supporting two things:

1. **Flox-supervised services** — the unit runs a FloxHub-managed
   environment and process management belongs entirely to Flox's services
   subsystem.
2. **Overrides of existing units** — swap the software an already-installed
   unit runs for Flox-provided versions, while tapping into the same
   declarative self-updating machinery as (1).

## What #2873 actually provides

Two ways to run a systemd service out of an activated Flox environment,
both backed by one shared provisioning mechanism:

1. **Services method** (`services.flox.activations.<name>`) — the unit's
   `ExecStart` is `flox activate --start-services -- flox services logs
   --follow`. Process supervision belongs to Flox (process-compose);
   systemd supervises a single log-follower in the foreground.
2. **Overrides method** (`systemd.services.<name>.flox`) — an existing
   unit keeps its own `[Service]` stanza (hardening included) and only its
   `ExecStart` is replaced with `flox activate -- <command>`.
3. **Pull machinery** (`pull.nix`) — `flox-pull@<name>.service` (ordered
   before the unit, `Requires=`) provisions on first start and refreshes
   on later starts; `flox-autopull@<name>.service` + a `.timer` refresh on
   a schedule and optionally `systemctl try-restart` the unit when the
   generation changed. A `flock` on `<workdir>/.flox-pull.lock` serialises
   the two.

Everything the units *do* at runtime is plain systemd + coreutils +
util-linux. The NixOS coupling is almost entirely in how the units are
*produced*.

## How the two goals map onto systemd mechanisms

**Goal 2 is the drop-in.** Drop-ins are systemd's mechanism for modifying a
unit you do not own. That is exactly the Overrides method: a vendor-shipped
`echoip.service` keeps its `[Service]` stanza — hardening, `User=`,
`Restart=`, ordering — and only what it executes is swapped.
`systemctl revert echoip.service` undoes it cleanly. It is the direct
analogue of `systemd.services.<name>.flox` being a *merge* into a unit
defined elsewhere.

**Goal 1 is a whole unit file.** There is no pre-existing unit to modify,
so we author `flox@.service` outright. A drop-in would have nothing to drop
into.

**The pull machinery is common to both, and is the larger half.**
`flox-pull@` and `flox-autopull@`+`.timer` are method-agnostic; both goals
hook in identically with `After=`/`Requires=flox-pull@<name>.service` —
for goal 1 those lines live in the unit we ship, for goal 2 they live in
the drop-in. The "declarative self-updating" the goals ask for is therefore
one implementation, not two.

## Inventory: what is NixOS-only

| Use | Where | Generic-Linux equivalent |
|---|---|---|
| Module system (`options`/`config`/`mkIf`/`mkMerge`/assertions/submodules) | all files | per-instance conf file + static template units |
| `pkgs.writeShellScript` (start script, ExecStart wrapper) | services.nix, overrides.nix, pull.nix | real scripts shipped in `/usr/libexec/flox/` |
| `pkgs.linkFarm "flox-pull-scripts"` (dir of per-service scripts indexed by `%i`) | pull.nix | **not needed** — one parametric script reading `/etc/flox/services/%i.conf` |
| `${package}/bin/flox` store path | everywhere | `/usr/bin/flox` (the `.deb`/`.rpm` install path) |
| `pkgs.runtimeShell` for `SHELL=` | common.nix | `/bin/sh` |
| `path = [ pkgs.coreutils pkgs.util-linux ]` | pull.nix | rely on `/usr/bin`, or an explicit `Environment=PATH=` |
| `utils.systemdUtils.lib.makeJobScript`, `config.script`, `config.scriptArgs`, `enableStrictShellChecks` | overrides.nix | no analogue — the `script`/`scriptArgs` half of the Overrides method does not port; only `execStart` does |
| `users.users` / `users.groups` declarative accounts | services.nix | `/usr/lib/sysusers.d/flox-<name>.conf` (systemd-sysusers) or `useradd -r` in the pull script |
| `systemd.tmpfiles.rules` | pull.nix | a real `/usr/lib/tmpfiles.d/flox.conf` — **directly portable** |
| `nix.settings` substituters/keys | modules/flox.nix | already handled by the `.deb`/`.rpm` installer's `/etc/nix/nix.conf` |
| `config.systemd.services.<name>` merge into a distro-shipped unit | overrides.nix | `/etc/systemd/system/<name>.service.d/10-flox.conf` drop-in with `ExecStart=` (reset) + `ExecStart=…` |

## The drop-in in practice (goal 2)

The NixOS Overrides method sells "reuse the hundreds of existing NixOS
service modules". On Debian/RHEL the equivalent asset is the **distro's own
unit files**, and systemd's native mechanism for what `mkForce
serviceConfig.ExecStart` does is a drop-in:

```ini
# /etc/systemd/system/echoip.service.d/10-flox.conf
[Unit]
After=flox-pull@echoip.service
Requires=flox-pull@echoip.service

[Service]
ExecStart=
ExecStart=/usr/libexec/flox/flox-exec-start echoip
Environment=HOME=/var/lib/flox/echoip …
ReadWritePaths=/var/lib/flox/echoip /nix/var/nix/daemon-socket
LoadCredential=floxhub_token:/run/keys/echoip.token
```

This is arguably a *better* story than the NixOS one: it works against any
vendor unit without rebuilding the system, and `systemctl revert
echoip.service` undoes it. The empty `ExecStart=` reset line is required;
units with multiple `ExecStart=` lines need the same reset and full
re-listing.

It is also already turnkey by distro convention — `systemctl edit
echoip.service` is the command an admin reaches for, and we can document
"paste this" with no tooling of our own.

## Runtime-portability gotchas (independent of packaging)

- **`LoadCredential=` / `$CREDENTIALS_DIRECTORY` needs systemd ≥ 247.**
  Fine on RHEL 9 (252), Debian 12 (252), Ubuntu 22.04 (249). **Not** on
  RHEL 8 (239) or Ubuntu 20.04 (245). Needs a fallback — likely
  `EnvironmentFile=` pointing at a root-owned `0600` file holding
  `FLOX_FLOXHUB_TOKEN=…`. See open question 1.
- **`setpriv --reuid --regid --init-groups`** needs util-linux ≥ 2.31 —
  RHEL 8 has 2.32, Debian 10 has 2.33, so this is safe everywhere
  currently supported. Worth an explicit floor in the packaging.
- **`systemd.tmpfiles`, template units, `%i`, timers with
  `Persistent=`/`RandomizedDelaySec=`, `flock`** — all portable, no
  changes needed.
- **Instance names**: `%i` is not unescaped (`%I` is). Service names with
  `-` are fine, but anything needing escaping must go through
  `systemd-escape`. Same latent issue exists in the NixOS module.
- **SELinux (RHEL/Fedora)**: units executing from `/nix/store` and writing
  under `/var/lib/flox` will need policy or `chcon`/`semanage fcontext`
  attention. The NixOS module never has to think about this. Likely the
  single biggest unknown — see open question 2.
- **AppArmor (Ubuntu)**: less likely to bite, but a vendor unit's existing
  profile may not cover `/nix/store` exec.
- **`ProtectSystem=strict` interaction**: the module already adds
  `ReadWritePaths=[workdir, /nix/var/nix/daemon-socket]`. On a generic
  distro the nix daemon socket path is the same, so this carries over
  verbatim.
- **`USER` must match the invoking uid's passwd entry** or `flox` resets
  `HOME` from the passwd db (see the comment in `common.nix`). The pinned
  `XDG_*` under the working directory is what makes that survivable. This
  constraint is a flox-CLI behaviour, fully portable, and must be kept.
- **Stale process-compose socket removal** (`rm -f
  <workdir>/.cache/flox/run/*.sock`) — portable, keep.

## Recommended shape: three layers, built in order

### Layer 0 — the units and scripts are the product

Static, parametric over `%i`, readable by a sysadmin who has never heard of
Nix:

- `/usr/lib/systemd/system/flox-pull@.service`, `flox-autopull@.service`,
  `flox-autopull@.timer`, `flox@.service` (goal 1's unit)
- `/usr/libexec/flox/flox-pull`, `flox-activate`, `flox-exec-start`
- `/usr/lib/tmpfiles.d/flox.conf`
- per-instance config in `/etc/flox/services/<name>.conf`
  (shell-sourced `KEY=value`)

`pull.nix`'s internal `services.flox.pull.configs` submodule is already
almost exactly that conf schema — it is a serialisable struct today. The
scripts become parametric over `$1` (the instance name) instead of being
generated per-service; `linkFarm` disappears.

These must be **hand-installable and hand-auditable**. That is what
"meeting users where they are" means for a systemd audience: they already
know template units, `/etc/`, and `systemctl edit`.

**Consequence for #2873:** `modules/nixos/` should install *these same
scripts* rather than carrying `writeShellScript` bodies. That removes the
duplicate-implementation risk, and `check.nix` plus one container test then
cover NixOS and generic Linux together — the port improves the NixOS module
rather than forking it.

### Layer 1 — ship them in the existing `.deb`/`.rpm`

Strongest turnkey answer, and it needs no new CLI surface. Template units
are inert without instances, so shipping them costs nothing. The user flow
becomes:

```sh
# goal 1
$EDITOR /etc/flox/services/echoip.conf
systemctl enable --now flox@echoip.service

# goal 2
systemctl edit echoip.service     # paste the drop-in above
```

### Layer 2 — a first-class `flox` subcommand, deferred

A convenience that writes the conf and the drop-in and runs
`daemon-reload`. When it lands it should be **core, not an extension**:
this is the README's "laptop to production" pitch, not a community add-on,
and it must be version-locked with the units in the same package.

Deferred rather than led with, for three reasons:

1. Every hard problem is in Layer 0 — SELinux, the systemd 247 floor,
   `ExecStart=` reset semantics, user creation. A Rust command makes none
   of them easier.
2. A generator writing into `/etc` needs an idempotency and ownership
   story (package upgrade, removal, admin hand-edits). Real design work,
   premature before the units are proven.
3. **Config-management users will not use a CLI generator.**
   Ansible/Puppet/Chef shops template files. Layer 0 serves them directly;
   a CLI-only story excludes a large slice of the production-Linux
   audience.

If ergonomics are wanted sooner, a shell `flox-systemd-setup` shipped in
the package is cheap to iterate on and can be ported to Rust once it
settles — at the cost of briefly having two implementations.

**Naming flag for Layer 2:** `flox services` currently means
process-compose services inside the *current activation*
(`start`/`stop`/`status`/`logs`/`restart`/`persist`, see
`cli/flox/src/commands/services/`). Nesting a host-level installer under it
— `flox services systemd install` — collides two very different scopes.

## Why not the extension subsystem

Verified in-tree at this base (`cli/flox/src/beta/extensions/`, single
commit `bafb98cea`):

- `flox extension install|list|remove` manages `flox-<name>` executables in
  `flox.data_dir/extensions`; `flox <name>` dispatches to them.
- Gated behind `features.beta`.
- **Local-directory installs only** — `install_local()`, reachable as
  `flox extension install .` or `--from-path PATH`. There is no remote,
  registry, or FloxHub source.

The last point rules it out as a delivery vehicle: a user would need the
extension source already on disk before installing it, which is the
opposite of turnkey. It also invites version drift between an out-of-band
extension and the units it manages.

Revisit if and when extensions gain a remote install source — a
*community* extension layering extra workflow on top of Layer 0 would be a
reasonable use, but the base capability should not depend on it.

## Resolved decisions

- **Where do the units ship?** In the **main `.deb`/`.rpm`**, not a
  separate `flox-systemd` package. Inert template units cost nothing, and
  a second package is one more thing to discover — anti-turnkey. The only
  new dependency is util-linux for `setpriv`, present everywhere.
- **Which method leads off-NixOS?** The **Overrides/drop-in method**. It is
  the idiomatic story for a distro audience, it needs no new supervision
  model, and `systemctl edit` + `systemctl revert` are commands admins
  already know. The Services method ships alongside it as `flox@.service`
  for users who want Flox to own supervision.
- **Where does config live and who owns the schema?**
  `/etc/flox/services/<name>.conf`, shell-sourced `KEY=value`, owned by
  Layer 0 and derived from the existing `pull.configs` submodule. A Layer 2
  subcommand generates that file rather than introducing a store of its
  own, so hand-editing and tooling stay interchangeable.

## Open questions

1. **Minimum supported systemd.** Does the `LoadCredential` floor of 247
   rule out RHEL 8 / Ubuntu 20.04, or do we ship the `EnvironmentFile`
   fallback? Affects whether the conf schema needs a token-delivery knob.
2. **SELinux.** Policy module, or documented `semanage fcontext` steps?
   Needs testing on Fedora/RHEL in enforcing mode before anything else is
   promised.
3. **Multi-`ExecStart` vendor units.** The drop-in reset requires
   re-listing every `ExecStart=`. Do we detect and refuse these, or
   document the manual path?

## Verification available today

- `nix flake check` builds `checks.<system>.nixos-module`
  (`modules/nixos/check.nix`) — eval-only, runs anywhere.
- There is no runtime test yet (no NixOS VM test, no bats coverage). The
  generic-Linux port needs a container/VM test from the start; per Layer 0
  above, that same harness closes the gap for the NixOS module too.
