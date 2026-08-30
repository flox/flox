# Shared helpers for the Flox systemd integration. Sourced, not executed.
#
# Every entry point takes a systemd instance name as its first argument and
# reads /etc/flox/services/<name>.conf for that instance's settings. Keeping
# the per-instance data in a conf file rather than in the unit is what lets
# the units stay static and installable by a package: systemd cannot read
# User=, ExecStart= or OnCalendar= out of a config file, but everything the
# scripts do can come from one.
#
# POSIX sh throughout - these run on whatever /bin/sh the distro ships.

FLOX_CONF_DIR=${FLOX_CONF_DIR:-/etc/flox/services}
FLOX_STATE_DIR=${FLOX_STATE_DIR:-/var/lib/flox}
FLOX_BIN=${FLOX_BIN:-/usr/bin/flox}

die() {
  echo "$*" >&2
  exit 1
}

# Populate the flox_* globals for instance $1 from its conf file.
#
# Defaults mirror the NixOS module: a per-service `flox-<name>` system user,
# a working directory under the state dir, and a refresh on every start.
#
# The FLOX_*_ARGS values stay as strings and are used unquoted at the call
# sites: they are operator-supplied argument lists, not filenames, and this
# is the conventional shape for a sysconfig-style file.
flox_load_conf() {
  flox_name=${1:?usage: flox_load_conf <instance>}

  flox_conf="$FLOX_CONF_DIR/$flox_name.conf"
  [ -r "$flox_conf" ] || die "no readable configuration at $flox_conf"

  FLOX_ENVIRONMENT=
  FLOX_USER=
  FLOX_GROUP=
  FLOX_TRUST=0
  FLOX_TOKEN_FILE=
  FLOX_PULL_AT_SERVICE_START=1
  FLOX_AUTORESTART=0
  FLOX_UNIT=
  FLOX_ARGS=
  FLOX_ACTIVATE_ARGS=
  FLOX_PULL_ARGS=
  FLOX_EXEC_START=

  # shellcheck disable=SC1090
  . "$flox_conf"

  [ -n "$FLOX_ENVIRONMENT" ] || die "FLOX_ENVIRONMENT is not set in $flox_conf"

  flox_env=$FLOX_ENVIRONMENT

  # As root (the system manager) each service gets its own `flox-<name>`
  # account, created on first pull. Unprivileged - a `systemd --user` service,
  # or a script run by hand - there is no account to create and no privilege
  # to drop, so the invoking user is the service user. Defaulting to
  # `flox-<name>` there would break the USER/passwd invariant below.
  if [ -n "$FLOX_USER" ]; then
    flox_user=$FLOX_USER
  elif [ "$(id -u)" = 0 ]; then
    flox_user=flox-$flox_name
  else
    flox_user=$(id -un)
  fi
  flox_group=${FLOX_GROUP:-$(flox_default_group)}
  flox_workdir="$FLOX_STATE_DIR/$flox_name"
  flox_token_file=$FLOX_TOKEN_FILE

  # The unit autoRestart restarts. Method 1 runs the instance as
  # flox@<name>.service; an override (method 2) is attached to a unit that
  # already exists under its own name, so those conf files set FLOX_UNIT.
  flox_unit=${FLOX_UNIT:-flox@$flox_name.service}
}

# Primary group for the service account: the matching per-service group as
# root, the invoking user's own group otherwise.
flox_default_group() {
  if [ "$(id -u)" = 0 ]; then
    echo "$flox_user"
  else
    id -gn
  fi
}

# Environment for every process that invokes flox on behalf of a service.
#
# USER must match the passwd name of the invoking uid: when it does not
# (e.g. under setpriv, or in a oneshot unit) flox resets HOME from the passwd
# database, discarding the working-directory HOME set here. The XDG variables
# are additionally pinned beneath the working directory so flox state stays
# with the service even if HOME is reset anyway. The pinned XDG_CONFIG_HOME
# gives each service an empty Flox configuration directory, so
# FLOX_DISABLE_METRICS is the only way the metrics setting can reach it.
#
# Emitted one per line for the caller to word-split into `env` arguments;
# none of the values can contain whitespace, since systemd instance names
# cannot.
flox_service_env() {
  printf '%s\n' \
    "FLOX_DISABLE_METRICS=${FLOX_DISABLE_METRICS:-false}" \
    "HOME=$flox_workdir" \
    "LOGNAME=$flox_user" \
    "SHELL=${SHELL:-/bin/sh}" \
    "USER=$flox_user" \
    "XDG_CACHE_HOME=$flox_workdir/.cache" \
    "XDG_CONFIG_HOME=$flox_workdir/.config" \
    "XDG_DATA_HOME=$flox_workdir/.local/share" \
    "XDG_STATE_HOME=$flox_workdir/.local/state"
}

# Export a FloxHub token if systemd passed one in as a credential.
#
# The token must only ever travel through the environment: putting it on a
# command line would expose it in /proc. LoadCredential= has no "optional"
# form, so the units never declare one and operators opt in with a drop-in;
# this stays quiet when they have not.
flox_export_token_from_credential() {
  if [ -n "${CREDENTIALS_DIRECTORY:-}" ] && [ -e "$CREDENTIALS_DIRECTORY/floxhub_token" ]; then
    FLOX_FLOXHUB_TOKEN="$(cat "$CREDENTIALS_DIRECTORY/floxhub_token")"
    export FLOX_FLOXHUB_TOKEN
  fi
}

# Remove a stale process-compose socket left by a previous instance.
#
# By the time systemd runs ExecStart the previous instance's cgroup is gone,
# so any socket in this service's private cache is stale. Left in place it
# prevents services from starting again after an unclean shutdown.
flox_clear_stale_sockets() {
  rm -f "$flox_workdir"/.cache/flox/run/*.sock
}
