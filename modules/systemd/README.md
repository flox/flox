# Flox systemd integration for non-NixOS Linux

Run services from FloxHub-managed environments on any distro that uses
systemd, with the same self-updating behavior as the NixOS module in
`../nixos`.

This is Layer 0 as described in [INVESTIGATION.md](./INVESTIGATION.md): static
template units plus per-instance config files, installable by hand or by a
distro package. No Nix evaluation is involved.

**New here?** Start with the [user guide](./docs/user-guide.md), which walks
through both methods on Ubuntu 24.04. This page is the reference.

## What gets installed

| Path | Purpose |
|---|---|
| `/usr/lib/systemd/system/flox@.service` | Runs a service entirely from a Flox environment |
| `/usr/lib/systemd/system/flox-pull@.service` | Provisions and refreshes an environment before its service starts |
| `/usr/lib/systemd/system/flox-autopull@.service` | Scheduled refresh |
| `/usr/lib/systemd/system/flox-autopull@.timer` | Schedule for the above |
| `/usr/libexec/flox/flox-{pull,activate,exec-start}` | The entry points the units call |
| `/usr/libexec/flox/flox-common.sh` | Shared helpers, sourced by the above |
| `/usr/lib/tmpfiles.d/flox.conf` | Creates `/var/lib/flox` |
| `/etc/flox/services/` | Operator config, one `<name>.conf` per service |

```sh
sudo make install
sudo systemctl daemon-reload
sudo systemd-tmpfiles --create
```

Packaging builds should use `make install DESTDIR=…`.

The template units are inert until an instance is enabled, so installing them
changes nothing on a system that is not using them.

## Method 1 — Flox owns supervision

`flox activate --start-services` starts the environment's services and
systemd supervises the log follower. Use this when the environment's
`[services]` section already describes everything that should run.

```sh
sudo cp examples/myservice.conf /etc/flox/services/echoip.conf
sudo $EDITOR /etc/flox/services/echoip.conf     # set FLOX_ENVIRONMENT
sudo systemctl enable --now flox@echoip.service
sudo systemctl enable --now flox-autopull@echoip.timer   # optional
```

`flox-pull@echoip.service` runs first, creating the `flox-echoip` account,
`/var/lib/flox/echoip`, and pulling the environment.

## Method 2 — override an existing unit

Keep a vendor unit's own `[Service]` stanza — hardening, `Restart=`,
ordering — and replace only what it executes. This is a systemd drop-in, so
`systemctl revert` undoes it.

```sh
sudo cp examples/echoip-override.conf /etc/flox/services/echoip.conf
sudo $EDITOR /etc/flox/services/echoip.conf     # set FLOX_EXEC_START, FLOX_USER
sudo systemctl edit echoip.service              # paste examples/echoip-override.dropin.conf
sudo systemctl restart echoip.service
```

The instance name must match the unit name: the drop-in passes it to
`flox-exec-start`, which reads `/etc/flox/services/<name>.conf`.

`FLOX_USER`/`FLOX_GROUP` must match the `User=`/`Group=` the vendor unit
already runs as, so the pull provisions the working directory for the right
account.

## Configuration reference

One `/etc/flox/services/<name>.conf` per service, shell-sourced `KEY=value`.

| Key | Default | Meaning |
|---|---|---|
| `FLOX_ENVIRONMENT` | *required* | The FloxHub environment to run |
| `FLOX_EXEC_START` | — | Method 2 only: the command to run in the environment |
| `FLOX_USER` / `FLOX_GROUP` | `flox-<name>` | Account to run as; created on first pull |
| `FLOX_TRUST` | `0` | Pass `--trust` when activating |
| `FLOX_TOKEN_FILE` | — | Root-readable file holding a FloxHub token, used for pulls |
| `FLOX_PULL_AT_SERVICE_START` | `1` | Refresh on every service start |
| `FLOX_AUTORESTART` | `0` | Restart the service when a scheduled pull fetches a new generation |
| `FLOX_ARGS` | — | Extra arguments for every `flox` invocation |
| `FLOX_ACTIVATE_ARGS` | — | Extra arguments for `flox activate` |
| `FLOX_PULL_ARGS` | — | Extra arguments for `flox pull` |

The three `*_ARGS` values hold a **shell-quoted argument list**, expanded with
`eval`. The simple case needs no thought — `FLOX_ARGS="-v -v"` is two
arguments — and an argument containing whitespace is quoted within the value:
`FLOX_ACTIVATE_ARGS="--mode 'dev mode'"` is `--mode` followed by `dev mode`.

`FLOX_STATE_DIR` (default `/var/lib/flox`), `FLOX_BIN` (default
`/usr/bin/flox`), `FLOX_CONF_DIR` and `FLOX_LIBEXEC` may be overridden in the
unit environment for testing.

**Quote any value containing whitespace.** The file is sourced by `/bin/sh`,
so `FLOX_EXEC_START=myserver --port 8080` is an assignment followed by an
attempt to run `--port`. Write `FLOX_EXEC_START="myserver --port 8080"`.

## Things systemd cannot read from the conf file

These are deliberately static in the units and changed with a drop-in
(`systemctl edit <unit>`), because systemd resolves them before any script
runs:

- **`User=` / `Group=`** — default `flox-%i`. Change both the drop-in and
  `FLOX_USER`/`FLOX_GROUP`, which must agree.
- **`OnCalendar=`** — default `daily` on `flox-autopull@.timer`. Reset with
  an empty `OnCalendar=` before the new value.
- **`LoadCredential=`** — not declared by default. `LoadCredential=` has no
  optional form, so declaring it in the template would make every instance
  without a token file fail to start. Add it per instance to give the
  *activation* a token; pulls do not need it, since `flox-pull` runs as root
  and reads `FLOX_TOKEN_FILE` directly.

## Requirements

- systemd. `LoadCredential=` needs 247+, but it is opt-in, so the rest works
  on 239 (RHEL 8).
- `setpriv` from util-linux 2.31+ (RHEL 8 ships 2.32).
- `flox` on the host, and a `/bin/sh`.

## Testing

```sh
make check      # parse the units, syntax-check the scripts
make test       # run the scripts against a stub flox (needs bats)
make test-user  # load the units into your own systemd --user manager
make test-all   # all three
```

`tests/scripts.bats` covers conf parsing, the argv each script assembles, the
pull/refresh/skip decisions, the asymmetric failure semantics between service
start and the timer, and restart-on-new-generation. It needs no systemd, no
root and no real `flox`, so it costs milliseconds.

`tests/user-manager.bats` covers what needs a real service manager: `%i`
instantiation, `Requires=`/`After=` ordering, the drop-in `ExecStart=` reset
against a unit that already exists, and the scheduled pull restarting its
unit. It transforms the units for a user manager with
`tests/mk-user-units.sh` — renamed `floxtest-*`, `User=`/`Group=` stripped,
paths redirected — and skips itself when no user manager is reachable.

Still uncovered, and only reachable with root on a real host: account
creation, `setpriv`, `/var/lib` paths, and SELinux.

## Not done yet

- SELinux is untested. Units executing from `/nix/store` and writing under
  `/var/lib/flox` will likely need policy work on RHEL/Fedora in enforcing
  mode. This needs a real host in enforcing mode, not a test harness.
- Multi-`ExecStart` vendor units need every `ExecStart=` re-listed after the
  drop-in's reset; nothing detects or warns about this.
- The scripts here duplicate logic that `../nixos` generates with
  `writeShellScript`. They should converge on one implementation.
