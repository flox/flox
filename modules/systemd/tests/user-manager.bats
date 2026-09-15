#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Service-manager tests for the Flox systemd units.
#
# These load the units into the caller's own `systemd --user` manager and
# actually start them. They cover what the stub-flox tests in scripts.bats
# cannot: %i instantiation, Requires=/After= ordering being honored, the
# drop-in ExecStart= reset against a unit that already exists, and timers.
#
# The units are transformed for the user manager by tests/mk-user-units.sh -
# renamed to `floxtest-*`, stripped of User=/Group=, and pointed at a temp
# directory and the stub flox. See that script for what fidelity is given up.
#
# Not covered here, because a user manager cannot do it: account creation,
# privilege dropping via setpriv, and anything under /var/lib.
#
# Skipped unless a user manager is running.
#
# ---------------------------------------------------------------------------- #

UNIT_DIR="${HOME}/.config/systemd/user"

setup_file() {
  # Not is-system-running: it reports failure for a merely degraded manager,
  # which is still perfectly able to run these units. Reaching the manager at
  # all is the real precondition.
  if ! systemctl --user show --property=Version >/dev/null 2>&1; then
    export FLOXTEST_NO_MANAGER=1
    return 0
  fi

  export FLOXTEST_ROOT="$(mktemp -d)"
  export FLOXTEST_STATE="$FLOXTEST_ROOT/state"
  export FLOXTEST_CONF="$FLOXTEST_ROOT/conf"
  export FLOXTEST_LOG="$FLOXTEST_STATE/invocations"
  mkdir -p "$FLOXTEST_STATE" "$FLOXTEST_CONF" "$UNIT_DIR"

  "${BATS_TEST_DIRNAME}/mk-user-units.sh" \
    "$UNIT_DIR" "$FLOXTEST_STATE" "$FLOXTEST_CONF" \
    "${BATS_TEST_DIRNAME}/../libexec" "${BATS_TEST_DIRNAME}/stub-flox"

  systemctl --user daemon-reload
}

teardown_file() {
  [ -n "${FLOXTEST_NO_MANAGER:-}" ] && return 0
  systemctl --user stop 'floxtest*' >/dev/null 2>&1 || true
  rm -f "$UNIT_DIR"/floxtest*
  systemctl --user daemon-reload || true
  rm -rf "${FLOXTEST_ROOT:?}"
}

setup() {
  [ -n "${FLOXTEST_NO_MANAGER:-}" ] && skip "no systemd --user manager running"
  systemctl --user reset-failed 'floxtest*' >/dev/null 2>&1 || true
  : > "$FLOXTEST_LOG"
}

write_conf() {
  local name="$1"; shift
  printf '%s\n' "$@" > "$FLOXTEST_CONF/$name.conf"
}

# ---------------------------------------------------------------------------- #
# Template instantiation
# ---------------------------------------------------------------------------- #

@test "the pull template instantiates per service and provisions it" {
  write_conf alpha "FLOX_ENVIRONMENT=flox/alpha"
  run systemctl --user start floxtest-pull@alpha.service
  [ "$status" -eq 0 ]
  # %i reached the script: it read alpha's conf and made alpha's directory.
  [ -d "$FLOXTEST_STATE/alpha" ]
  grep -q '^arg=flox/alpha$' "$FLOXTEST_LOG"
}

@test "two instances of the template stay independent" {
  write_conf one "FLOX_ENVIRONMENT=flox/one"
  write_conf two "FLOX_ENVIRONMENT=flox/two"
  systemctl --user start floxtest-pull@one.service
  systemctl --user start floxtest-pull@two.service
  [ -d "$FLOXTEST_STATE/one" ]
  [ -d "$FLOXTEST_STATE/two" ]
  grep -q '^arg=flox/one$' "$FLOXTEST_LOG"
  grep -q '^arg=flox/two$' "$FLOXTEST_LOG"
}

@test "an instance with no conf file fails rather than starting empty" {
  run systemctl --user start floxtest-pull@nosuch.service
  [ "$status" -ne 0 ]
}

# ---------------------------------------------------------------------------- #
# Ordering: the environment must exist before anything runs from it
# ---------------------------------------------------------------------------- #

@test "the service pulls its dependency in and runs after it" {
  write_conf beta "FLOX_ENVIRONMENT=flox/beta"
  run systemctl --user start floxtest@beta.service
  [ "$status" -eq 0 ]

  # Requires= started the pull without it being named on the command line.
  run systemctl --user is-active floxtest-pull@beta.service
  [ "$output" = "active" ] || [ "$output" = "inactive" ]

  # After= put it first: the pull's invocation is logged before the activate.
  pull_line=$(grep -n '^arg=pull$' "$FLOXTEST_LOG" | head -1 | cut -d: -f1)
  activate_line=$(grep -n '^arg=activate$' "$FLOXTEST_LOG" | head -1 | cut -d: -f1)
  [ -n "$pull_line" ]
  [ -n "$activate_line" ]
  [ "$pull_line" -lt "$activate_line" ]
}

@test "a failed provision keeps the service from starting" {
  write_conf gamma "FLOX_ENVIRONMENT=flox/gamma" "FLOX_TOKEN_FILE=/nonexistent"
  run systemctl --user start floxtest@gamma.service
  [ "$status" -ne 0 ]
  # Requires= propagated the failure: the activation never ran.
  ! grep -q '^arg=activate$' "$FLOXTEST_LOG"
}

# ---------------------------------------------------------------------------- #
# Method 2: a drop-in replacing an existing unit's ExecStart
# ---------------------------------------------------------------------------- #

@test "a drop-in replaces the vendor unit's command with the Flox activation" {
  # Stand in for a distro-shipped unit that already exists on the system.
  cat > "$UNIT_DIR/floxtest-vendor.service" <<UNIT
[Unit]
Description=Floxtest vendor unit
[Service]
Type=oneshot
ExecStart=/bin/echo vendor-command-ran
UNIT
  mkdir -p "$UNIT_DIR/floxtest-vendor.service.d"
  cat > "$UNIT_DIR/floxtest-vendor.service.d/10-flox.conf" <<UNIT
[Unit]
After=floxtest-pull@floxtest-vendor.service
Requires=floxtest-pull@floxtest-vendor.service
[Service]
Environment=FLOX_CONF_DIR=$FLOXTEST_CONF
Environment=FLOX_STATE_DIR=$FLOXTEST_STATE
Environment=FLOX_LIBEXEC=${BATS_TEST_DIRNAME}/../libexec
Environment=FLOX_BIN=${BATS_TEST_DIRNAME}/stub-flox
Environment=FLOX_STUB_LOG=$FLOXTEST_LOG
ExecStart=
ExecStart=${BATS_TEST_DIRNAME}/../libexec/flox-exec-start floxtest-vendor
UNIT
  systemctl --user daemon-reload

  write_conf floxtest-vendor \
    "FLOX_ENVIRONMENT=flox/vendor" \
    "FLOX_EXEC_START='vendor-server --port 9999'"

  run systemctl --user start floxtest-vendor.service
  [ "$status" -eq 0 ]

  # The reset took effect: the activation ran, the vendor command did not.
  grep -q '^arg=activate$' "$FLOXTEST_LOG"
  grep -q '^arg=vendor-server --port 9999$' "$FLOXTEST_LOG"
  run journalctl --user -u floxtest-vendor.service --since "-1 min" --no-pager
  [[ "$output" != *"vendor-command-ran"* ]]

  rm -rf "$UNIT_DIR/floxtest-vendor.service" "$UNIT_DIR/floxtest-vendor.service.d"
  systemctl --user daemon-reload
}

# ---------------------------------------------------------------------------- #
# Timers
# ---------------------------------------------------------------------------- #

@test "the autopull timer is loadable and schedules its service" {
  write_conf delta "FLOX_ENVIRONMENT=flox/delta"
  run systemctl --user start floxtest-autopull@delta.timer
  [ "$status" -eq 0 ]
  run systemctl --user show floxtest-autopull@delta.timer -p Unit --value
  [ "$output" = "floxtest-autopull@delta.service" ]
  systemctl --user stop floxtest-autopull@delta.timer
}

@test "the scheduled pull runs in timer mode and can restart its unit" {
  write_conf epsilon \
    "FLOX_ENVIRONMENT=flox/epsilon" \
    "FLOX_AUTORESTART=1" \
    "FLOX_UNIT=floxtest@epsilon.service"

  # Mark the environment provisioned. The stub does not create .flox, and an
  # unprovisioned pull takes the first-provision branch and returns before it
  # ever considers a restart.
  mkdir -p "$FLOXTEST_STATE/epsilon/.flox"

  # try-restart is a no-op on an inactive unit, so the activation has to stay
  # up for a restart to be observable at all.
  mkdir -p "$UNIT_DIR/floxtest@epsilon.service.d"
  cat > "$UNIT_DIR/floxtest@epsilon.service.d/10-stay.conf" <<UNIT
[Service]
Environment=FLOX_STUB_SLEEP=300
UNIT
  # A changing generation makes the pull decide a restart is warranted.
  mkdir -p "$UNIT_DIR/floxtest-autopull@epsilon.service.d"
  cat > "$UNIT_DIR/floxtest-autopull@epsilon.service.d/10-gen.conf" <<UNIT
[Service]
Environment=FLOX_STUB_GENERATION_CHANGES=1
UNIT
  systemctl --user daemon-reload

  systemctl --user start floxtest@epsilon.service
  run systemctl --user is-active floxtest@epsilon.service
  [ "$output" = "active" ]
  before=$(systemctl --user show floxtest@epsilon.service -p InvocationID --value)

  run systemctl --user start floxtest-autopull@epsilon.service
  [ "$status" -eq 0 ]

  # The restart is dispatched with --no-block to avoid deadlocking against
  # this script's own lock, so wait for the manager to get to it.
  after=$before
  for _ in $(seq 50); do
    after=$(systemctl --user show floxtest@epsilon.service -p InvocationID --value)
    [ "$before" != "$after" ] && break
    sleep 0.2
  done

  # try-restart reached the right unit through the user manager.
  [ -n "$before" ]
  [ "$before" != "$after" ]

  systemctl --user stop floxtest@epsilon.service
  rm -rf "$UNIT_DIR/floxtest@epsilon.service.d" \
         "$UNIT_DIR/floxtest-autopull@epsilon.service.d"
  systemctl --user daemon-reload
}
