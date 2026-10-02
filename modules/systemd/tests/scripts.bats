#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Tests for the Flox systemd entry-point scripts.
#
# These exercise the scripts directly with a stub `flox` on FLOX_BIN and the
# conf and state directories redirected into a temp dir. No systemd, no root
# and no real flox are involved: the point is to pin down conf parsing, the
# argv the scripts assemble, and the pull/refresh/restart decision logic.
#
# Unit semantics (%i instantiation, ordering, drop-in ExecStart= resets,
# timers) are not covered here - those need a running service manager.
#
# ---------------------------------------------------------------------------- #

setup() {
  LIBEXEC="${BATS_TEST_DIRNAME}/../libexec"
  export FLOX_CONF_DIR="${BATS_TEST_TMPDIR}/conf"
  export FLOX_STATE_DIR="${BATS_TEST_TMPDIR}/state"
  export FLOX_BIN="${BATS_TEST_DIRNAME}/stub-flox"
  export FLOX_STUB_LOG="${BATS_TEST_TMPDIR}/invocations"
  export FLOX_STUB_SYSTEMCTL_LOG="${BATS_TEST_TMPDIR}/systemctl"
  mkdir -p "$FLOX_CONF_DIR" "$FLOX_STATE_DIR"
  : > "$FLOX_STUB_LOG"
  : > "$FLOX_STUB_SYSTEMCTL_LOG"

  # The stub systemctl shadows any real one for try-restart assertions.
  mkdir -p "${BATS_TEST_TMPDIR}/bin"
  ln -sf "${BATS_TEST_DIRNAME}/stub-systemctl" "${BATS_TEST_TMPDIR}/bin/systemctl"
  PATH="${BATS_TEST_TMPDIR}/bin:$PATH"
}

# Write a conf file for instance $1 from the remaining arguments.
write_conf() {
  local name="$1"; shift
  printf '%s\n' "$@" > "$FLOX_CONF_DIR/$name.conf"
}

# Mark an instance's environment as already provisioned.
provision() {
  mkdir -p "$FLOX_STATE_DIR/$1/.flox"
}

# All arguments the stub was invoked with, across every invocation.
stub_args() {
  grep '^arg=' "$FLOX_STUB_LOG" | sed 's/^arg=//'
}

stub_invocations() {
  grep -c '^---$' "$FLOX_STUB_LOG" || true
}

# ---------------------------------------------------------------------------- #
# Configuration loading
# ---------------------------------------------------------------------------- #

@test "pull: missing conf file is an error naming the path" {
  run "$LIBEXEC/flox-pull" nosuch start
  [ "$status" -ne 0 ]
  [[ "$output" == *"$FLOX_CONF_DIR/nosuch.conf"* ]]
}

@test "pull: conf without FLOX_ENVIRONMENT is an error" {
  write_conf svc "FLOX_TRUST=1"
  run "$LIBEXEC/flox-pull" svc start
  [ "$status" -ne 0 ]
  [[ "$output" == *"FLOX_ENVIRONMENT"* ]]
}

@test "unprivileged run refuses a FLOX_USER that is not the invoking user" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_USER=someone-else"
  run "$LIBEXEC/flox-pull" svc start
  [ "$status" -ne 0 ]
  [[ "$output" == *"not running as root"* ]]
}

# ---------------------------------------------------------------------------- #
# Provisioning and refresh
# ---------------------------------------------------------------------------- #

@test "pull: first start provisions with a plain pull of the environment" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc"
  run "$LIBEXEC/flox-pull" svc start
  [ "$status" -eq 0 ]
  run stub_args
  [[ "$output" == *"pull"* ]]
  [[ "$output" == *"flox/svc"* ]]
  # A first provision must not use --force: there is nothing to overwrite.
  [[ "$output" != *"--force"* ]]
}

@test "pull: an already-provisioned environment refreshes with --force" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc"
  provision svc
  run "$LIBEXEC/flox-pull" svc start
  [ "$status" -eq 0 ]
  run stub_args
  [[ "$output" == *"--force"* ]]
}

@test "pull: FLOX_PULL_AT_SERVICE_START=0 skips the refresh on start" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_PULL_AT_SERVICE_START=0"
  provision svc
  run "$LIBEXEC/flox-pull" svc start
  [ "$status" -eq 0 ]
  [ "$(stub_invocations)" -eq 0 ]
}

@test "pull: FLOX_PULL_AT_SERVICE_START=0 still refreshes on the timer" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_PULL_AT_SERVICE_START=0"
  provision svc
  run "$LIBEXEC/flox-pull" svc timer
  [ "$status" -eq 0 ]
  run stub_args
  [[ "$output" == *"--force"* ]]
}

@test "pull: extra argument lists reach the right invocations" {
  write_conf svc \
    "FLOX_ENVIRONMENT=flox/svc" \
    "FLOX_ARGS='-v -v'" \
    "FLOX_PULL_ARGS=--dry-run"
  provision svc
  run "$LIBEXEC/flox-pull" svc start
  [ "$status" -eq 0 ]
  run stub_args
  [[ "$output" == *"-v"* ]]
  [[ "$output" == *"--dry-run"* ]]
}

@test "pull: a token file is exported to flox rather than passed as an argument" {
  echo "s3cr3t" > "${BATS_TEST_TMPDIR}/token"
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_TOKEN_FILE=${BATS_TEST_TMPDIR}/token"
  run "$LIBEXEC/flox-pull" svc start
  [ "$status" -eq 0 ]
  grep -q '^token=s3cr3t$' "$FLOX_STUB_LOG"
  run stub_args
  [[ "$output" != *"s3cr3t"* ]]
}

@test "pull: an unreadable token file is an error" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_TOKEN_FILE=${BATS_TEST_TMPDIR}/absent"
  run "$LIBEXEC/flox-pull" svc start
  [ "$status" -ne 0 ]
  [[ "$output" == *"not readable"* ]]
}

# ---------------------------------------------------------------------------- #
# Failure semantics: a failed refresh must not block a service start, but
# must be reported when the timer runs it.
# ---------------------------------------------------------------------------- #

@test "pull: a failed refresh on start warns and succeeds" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc"
  provision svc
  FLOX_STUB_EXIT=1 run "$LIBEXEC/flox-pull" svc start
  [ "$status" -eq 0 ]
  [[ "$output" == *"WARNING"* ]]
}

@test "pull: a failed refresh on the timer fails" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc"
  provision svc
  FLOX_STUB_EXIT=1 run "$LIBEXEC/flox-pull" svc timer
  [ "$status" -ne 0 ]
  [[ "$output" == *"ERROR"* ]]
}

@test "pull: a failed first provision fails" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc"
  FLOX_STUB_EXIT=1 run "$LIBEXEC/flox-pull" svc start
  [ "$status" -ne 0 ]
}

# ---------------------------------------------------------------------------- #
# Restart on a new generation
# ---------------------------------------------------------------------------- #

@test "pull: autorestart restarts the unit when the generation changed" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_AUTORESTART=1"
  provision svc
  # The stub reports a different generation on each call, so the fingerprint
  # taken before the pull differs from the one taken after.
  FLOX_STUB_GENERATION_CHANGES=1 run "$LIBEXEC/flox-pull" svc timer
  [ "$status" -eq 0 ]
  [[ "$output" == *"changed"* ]]
  grep -q "try-restart flox@svc.service" "$FLOX_STUB_SYSTEMCTL_LOG"
}

@test "pull: a changed generation does not restart on service start" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_AUTORESTART=1"
  provision svc
  # A start-mode pull is already followed by the service starting; restarting
  # it here would be redundant, and at boot would fight the ordering.
  FLOX_STUB_GENERATION_CHANGES=1 run "$LIBEXEC/flox-pull" svc start
  [ "$status" -eq 0 ]
  [ ! -s "$FLOX_STUB_SYSTEMCTL_LOG" ]
}

@test "pull: autorestart is quiet when the generation is unchanged" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_AUTORESTART=1"
  provision svc
  FLOX_STUB_GENERATION=stable run "$LIBEXEC/flox-pull" svc timer
  [ "$status" -eq 0 ]
  [ ! -s "$FLOX_STUB_SYSTEMCTL_LOG" ]
}

@test "pull: without autorestart the unit is never restarted" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc"
  provision svc
  run "$LIBEXEC/flox-pull" svc timer
  [ "$status" -eq 0 ]
  [ ! -s "$FLOX_STUB_SYSTEMCTL_LOG" ]
}

# ---------------------------------------------------------------------------- #
# flox-activate: method 1, Flox owns supervision
# ---------------------------------------------------------------------------- #

@test "activate: starts services and follows their logs" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc"
  provision svc
  run "$LIBEXEC/flox-activate" svc
  [ "$status" -eq 0 ]
  run stub_args
  [[ "$output" == *"activate"* ]]
  [[ "$output" == *"--start-services"* ]]
  [[ "$output" == *"--dir"* ]]
}

@test "activate: FLOX_TRUST=1 passes --trust" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_TRUST=1"
  provision svc
  run "$LIBEXEC/flox-activate" svc
  [ "$status" -eq 0 ]
  run stub_args
  [[ "$output" == *"--trust"* ]]
}

@test "activate: --trust is absent by default" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc"
  provision svc
  run "$LIBEXEC/flox-activate" svc
  [ "$status" -eq 0 ]
  run stub_args
  [[ "$output" != *"--trust"* ]]
}

@test "activate: extra activate arguments are passed through" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_ACTIVATE_ARGS='--mode dev'"
  provision svc
  run "$LIBEXEC/flox-activate" svc
  [ "$status" -eq 0 ]
  run stub_args
  [[ "$output" == *"--mode"* ]]
  [[ "$output" == *"dev"* ]]
}

@test "activate: a stale process-compose socket is removed first" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc"
  provision svc
  mkdir -p "$FLOX_STATE_DIR/svc/.cache/flox/run"
  touch "$FLOX_STATE_DIR/svc/.cache/flox/run/stale.sock"
  run "$LIBEXEC/flox-activate" svc
  [ "$status" -eq 0 ]
  [ ! -e "$FLOX_STATE_DIR/svc/.cache/flox/run/stale.sock" ]
}

# ---------------------------------------------------------------------------- #
# flox-exec-start: method 2, systemd keeps supervision
# ---------------------------------------------------------------------------- #

@test "exec-start: runs the configured command in the environment" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_EXEC_START='echoip -l 127.0.0.1:8080'"
  provision svc
  run "$LIBEXEC/flox-exec-start" svc
  [ "$status" -eq 0 ]
  run stub_args
  [[ "$output" == *"activate"* ]]
  [[ "$output" == *"echoip -l 127.0.0.1:8080"* ]]
  # Method 2 leaves supervision to systemd, so it must not start Flox services.
  [[ "$output" != *"--start-services"* ]]
}

@test "exec-start: without FLOX_EXEC_START it is an error" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc"
  provision svc
  run "$LIBEXEC/flox-exec-start" svc
  [ "$status" -ne 0 ]
  [[ "$output" == *"FLOX_EXEC_START"* ]]
}

# ---------------------------------------------------------------------------- #
# The service environment flox is invoked with
# ---------------------------------------------------------------------------- #

@test "flox runs with HOME and USER pinned to the service's own identity" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc"
  run "$LIBEXEC/flox-pull" svc start
  [ "$status" -eq 0 ]
  grep -q "^home=$FLOX_STATE_DIR/svc$" "$FLOX_STUB_LOG"
  grep -q "^user=$(id -un)$" "$FLOX_STUB_LOG"
}

@test "conf values containing whitespace must be quoted to survive sourcing" {
  # Unquoted, `FLOX_EXEC_START=a b` is an assignment followed by the command
  # `b`, which is why every multi-word value in the examples is quoted.
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_EXEC_START='one two three'"
  provision svc
  run "$LIBEXEC/flox-exec-start" svc
  [ "$status" -eq 0 ]
  run stub_args
  [[ "$output" == *"one two three"* ]]
}

@test "pull: FLOX_UNIT overrides which unit autorestart restarts" {
  # An override (method 2) is attached to a unit that already exists under its
  # own name, rather than to flox@<name>.service.
  write_conf svc \
    "FLOX_ENVIRONMENT=flox/svc" \
    "FLOX_AUTORESTART=1" \
    "FLOX_UNIT=echoip.service"
  provision svc
  FLOX_STUB_GENERATION_CHANGES=1 run "$LIBEXEC/flox-pull" svc timer
  [ "$status" -eq 0 ]
  grep -q "try-restart echoip.service" "$FLOX_STUB_SYSTEMCTL_LOG"
}

@test "activate: the service environment is pinned for the activation too" {
  # The units do not set HOME/USER/XDG_*; the entry points apply them, so the
  # NixOS module and the plain units cannot drift apart.
  write_conf svc "FLOX_ENVIRONMENT=flox/svc"
  provision svc
  HOME=/somewhere/else run "$LIBEXEC/flox-activate" svc
  [ "$status" -eq 0 ]
  grep -q "^home=$FLOX_STATE_DIR/svc$" "$FLOX_STUB_LOG"
  grep -q "^user=$(id -un)$" "$FLOX_STUB_LOG"
}

@test "exec-start: the service environment is pinned for the command too" {
  write_conf svc "FLOX_ENVIRONMENT=flox/svc" "FLOX_EXEC_START=true"
  provision svc
  HOME=/somewhere/else run "$LIBEXEC/flox-exec-start" svc
  [ "$status" -eq 0 ]
  grep -q "^home=$FLOX_STATE_DIR/svc$" "$FLOX_STUB_LOG"
}

@test "FLOX_CONF_FILE names the configuration directly" {
  # A caller with one unit per service points at the file rather than relying
  # on <instance>.conf being present in a directory.
  printf '%s\n' "FLOX_ENVIRONMENT=flox/elsewhere" > "$BATS_TEST_TMPDIR/other.conf"
  FLOX_CONF_FILE="$BATS_TEST_TMPDIR/other.conf" run "$LIBEXEC/flox-pull" svc start
  [ "$status" -eq 0 ]
  run stub_args
  [[ "$output" == *"flox/elsewhere"* ]]
}

@test "an argument containing whitespace survives to flox" {
  # The lists are shell-quoted in the conf file and expanded with eval, so a
  # quoted argument reaches flox as one argument rather than two.
  write_conf svc \
    "FLOX_ENVIRONMENT=flox/svc" \
    "FLOX_ACTIVATE_ARGS=\"--mode 'dev mode'\""
  provision svc
  run "$LIBEXEC/flox-activate" svc
  [ "$status" -eq 0 ]
  grep -q "^arg=dev mode$" "$FLOX_STUB_LOG"
}

@test "a whitespace argument survives the pull path too" {
  write_conf svc \
    "FLOX_ENVIRONMENT=flox/svc" \
    "FLOX_PULL_ARGS=\"--note 'two words'\""
  run "$LIBEXEC/flox-pull" svc start
  [ "$status" -eq 0 ]
  grep -q "^arg=two words$" "$FLOX_STUB_LOG"
}
