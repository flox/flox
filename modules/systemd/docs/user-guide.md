# Running Flox services under systemd

This guide walks through running services from FloxHub-managed environments
on a systemd Linux distribution, using Ubuntu 24.04 LTS as the worked
example. The mechanism is plain systemd, so the same steps apply on Debian,
Fedora and RHEL with only the package-install command changing.

You get two things:

- **Services that Flox runs end to end** — the environment's own `[services]`
  section describes what runs, and systemd keeps it alive.
- **Existing services running Flox-provided software** — an already-installed
  unit keeps its own hardening, restart policy and ordering, and only the
  software it executes comes from a Flox environment.

Both refresh themselves: the environment is pulled before the service starts,
on a schedule you choose, and the service can restart itself when a pull
brings in a new generation.

## Before you start

On Ubuntu 24.04 you already have everything the integration needs:

```console
$ systemctl --version | head -1
systemd 255 (255.4-1ubuntu8.17)
$ setpriv --version
setpriv from util-linux 2.39.3
```

Install Flox itself from the `.deb` package if you have not already — see
<https://flox.dev/docs/install-flox/install/>. Then install the systemd
integration:

```console
$ sudo make install
$ sudo systemctl daemon-reload
$ sudo systemd-tmpfiles --create
```

This adds four template units under `/usr/lib/systemd/system/`, their
supporting scripts under `/usr/libexec/flox/`, and an empty
`/etc/flox/services/` for your configuration. Template units do nothing until
you enable an instance, so installing them changes nothing yet.

You will also want a FloxHub token if your environments are private. Put it
in a root-owned file:

```console
$ sudo install -m 0600 /dev/null /etc/flox/services/echoip.token
$ sudo $EDITOR /etc/flox/services/echoip.token
```

## Method 1 — let Flox run the service

Use this when the Flox environment already describes what should run in its
`[services]` section. Flox starts those processes and systemd supervises the
result.

Describe the service in `/etc/flox/services/<name>.conf`, where `<name>` is
whatever you want the systemd instance to be called:

```sh
# /etc/flox/services/echoip.conf
FLOX_ENVIRONMENT=flox/echoip
FLOX_TRUST=1
FLOX_TOKEN_FILE=/etc/flox/services/echoip.token
FLOX_AUTORESTART=1
```

Then start it:

```console
$ sudo systemctl enable --now flox@echoip.service
```

That single command does rather a lot. `flox@echoip.service` requires
`flox-pull@echoip.service`, which runs first as root and:

1. creates a dedicated `flox-echoip` system account,
2. creates `/var/lib/flox/echoip` owned by it,
3. drops to that account and runs `flox pull flox/echoip` into it.

Only then does the service itself start, activating the environment with
`--start-services` and following its logs.

Check on it the way you would any other unit:

```console
$ systemctl status flox@echoip.service
$ journalctl -u flox@echoip.service -f
```

To keep the environment up to date, enable the timer as well:

```console
$ sudo systemctl enable --now flox-autopull@echoip.timer
```

It pulls daily by default. Because the conf file above sets
`FLOX_AUTORESTART=1`, a pull that fetches a new generation also restarts the
service; without it, an update waits for the next restart.

To pull on a different schedule, override the timer — systemd cannot read
`OnCalendar=` from the conf file, so it lives in a drop-in:

```console
$ sudo systemctl edit flox-autopull@echoip.timer
```

```ini
[Timer]
OnCalendar=
OnCalendar=hourly
```

The empty `OnCalendar=` resets the default before setting the new value.
Every list-valued systemd setting works this way.

## Method 2 — run an existing service's software from Flox

Use this when a unit already exists — from an Ubuntu package or your own
configuration management — and you want to keep it exactly as it is while
changing which build of the software it runs. The unit's `User=`,
`Restart=`, `ProtectSystem=` and ordering all stay untouched.

Start by reading the unit you are about to modify:

```console
$ systemctl cat prometheus-node-exporter.service
```

Note its `User=`, `Group=` and `ExecStart=`. Then write the conf file, using
the unit's own name as the instance name:

```sh
# /etc/flox/services/prometheus-node-exporter.conf
FLOX_ENVIRONMENT=flox/node-exporter
FLOX_TRUST=1
FLOX_EXEC_START="node_exporter --web.listen-address=127.0.0.1:9100"
FLOX_USER=prometheus
FLOX_GROUP=prometheus
FLOX_UNIT=prometheus-node-exporter.service
FLOX_AUTORESTART=1
```

`FLOX_USER` and `FLOX_GROUP` must match what the unit already runs as, so the
environment is provisioned for the right account. `FLOX_UNIT` names the unit
to restart after a pull — for an override that is the existing unit, not
`flox@...`.

Now attach the drop-in:

```console
$ sudo systemctl edit prometheus-node-exporter.service
```

```ini
[Unit]
After=flox-pull@prometheus-node-exporter.service
Requires=flox-pull@prometheus-node-exporter.service

[Service]
ExecStart=
ExecStart=/usr/libexec/flox/flox-exec-start prometheus-node-exporter
ReadWritePaths=/var/lib/flox/prometheus-node-exporter /nix/var/nix/daemon-socket
```

```console
$ sudo systemctl restart prometheus-node-exporter.service
```

The empty `ExecStart=` is required: it clears the vendor's command so yours
replaces it rather than being appended. The `ReadWritePaths=` line matters
because units like this are commonly hardened with `ProtectSystem=strict`,
and the activation still needs to write its working directory and reach the
Nix daemon.

Everything is reversible:

```console
$ sudo systemctl revert prometheus-node-exporter.service
```

### Units this does not fit

Check `systemctl cat` output for two things before you start:

- **Several `ExecStart=` lines.** The reset clears all of them, so you must
  re-list every one you still want. Nothing warns you about this.
- **`ExecStartPre=` running the distro's own binary**, as `nginx.service`
  does when it validates the config with the packaged `nginx -t`. Those lines
  are not reset and will still run the distro build. Either override them too
  or use method 1 instead.

## Configuration reference

See the table in [../README.md](../README.md#configuration-reference).

Two rules are worth repeating because they cause confusing failures:

- **Quote any value containing whitespace.** The file is sourced by `/bin/sh`,
  so `FLOX_EXEC_START=server --port 80` is an assignment followed by an
  attempt to run `--port`.
- **`FLOX_ARGS`, `FLOX_ACTIVATE_ARGS` and `FLOX_PULL_ARGS` are shell-quoted
  argument lists.** `FLOX_ARGS="-v -v"` is two arguments; an argument that
  itself contains whitespace is quoted inside the value, as in
  `FLOX_ACTIVATE_ARGS="--mode 'dev mode'"`.
- **`User=`, `Group=`, `ExecStart=`, `OnCalendar=` and `LoadCredential=`
  cannot be read from the conf file.** systemd resolves them before any of
  our scripts run. Change them with `systemctl edit`.

## Giving the activation a token

`flox-pull` runs as root and reads `FLOX_TOKEN_FILE` directly, so pulls work
with the conf file alone. The service itself runs as an unprivileged account
and cannot read that file. If your activation needs the token too, pass it as
a systemd credential:

```console
$ sudo systemctl edit flox@echoip.service
```

```ini
[Service]
LoadCredential=floxhub_token:/etc/flox/services/echoip.token
```

This is opt-in rather than built into the units because `LoadCredential=` has
no optional form: a unit that declares it fails to start when the file is
absent, which would break every instance that does not use a token.

## Troubleshooting

**The service fails immediately, with nothing useful in its own logs.** Look
at the pull unit instead — it runs first and most first-run problems are
there:

```console
$ journalctl -u flox-pull@echoip.service -n 50
```

**`no readable configuration at /etc/flox/services/<name>.conf`.** The
instance name and the conf filename must match exactly.

**`-l: command not found`, or an argument going missing.** An unquoted
multi-word value in the conf file. See the quoting rule above.

**The environment pulls but the service will not activate.** Try
`FLOX_TRUST=1`. Environments with hooks require it, and a non-interactive
activation cannot prompt.

**Reproduce a failure by hand.** The scripts are ordinary shell and take the
instance name as an argument:

```console
$ sudo /usr/libexec/flox/flox-pull echoip start
```

## Running services as yourself

The same scripts work unprivileged, which is useful on a workstation where
you want a Flox environment supervised without touching system state. Point
them at your own directories and use a user unit:

```console
$ mkdir -p ~/.config/flox/services ~/.local/share/flox
```

Set `FLOX_CONF_DIR`, `FLOX_STATE_DIR` and `FLOX_LIBEXEC` in the unit's
`Environment=`, and omit `User=`/`Group=` — a user manager cannot change uid,
and the service account is simply you. `tests/mk-user-units.sh` in this
repository generates exactly such a set of units and is a working reference.

## Ubuntu-specific notes

**AppArmor.** Ubuntu confines some packaged services with AppArmor profiles.
A profile written for the distro build may not permit executing from
`/nix/store`. If an override fails with a permission error that `ls -l` says
should have worked, check `journalctl -k | grep DENIED`.

**Unattended upgrades** replacing a unit file do not disturb your drop-in:
drop-ins live in `/etc/systemd/system/<unit>.d/` and survive package
upgrades. They can, however, start conflicting with a vendor unit whose
`ExecStart=` has changed shape — worth re-checking `systemctl cat` after a
major upgrade of the packaged service.
