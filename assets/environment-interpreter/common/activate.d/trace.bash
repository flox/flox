#!/bin/bash
set -o pipefail

# Chrome Trace Event JSON profiling support.
# When FLOX_PROFILE is set and _FLOX_PROFILE_DIR is available,
# writes B/E (begin/end) phase markers to a NDJSON file.
_flox_profile_trace() {
  [ -n "${FLOX_PROFILE:-}" ] && [ -n "${_FLOX_PROFILE_DIR:-}" ] || return 0

  local phase="$1"
  local name="$2"
  local ts_us

  # Get timestamp in microseconds
  if ts_ns=$(date +%s%N 2>/dev/null) && [ "${#ts_ns}" -gt 10 ]; then
    ts_us=$(( ts_ns / 1000 ))
  elif command -v perl >/dev/null 2>&1; then
    ts_us=$(perl -MTime::HiRes=gettimeofday -e '($s,$us)=gettimeofday;print $s*1000000+$us')
  else
    # Fallback: seconds precision
    ts_us=$(( $(date +%s) * 1000000 ))
  fi

  # Use PPID (the activate script's PID) so all events share the same
  # pid/tid. Using $$ would give this subprocess's PID which differs
  # per invocation, preventing Perfetto from matching B/E pairs.
  local pid=${PPID:-1}
  local tid=${PPID:-1}

  printf '{"ph":"%s","name":"%s","pid":%d,"tid":%d,"ts":%s}\n' \
    "$phase" "$name" "$pid" "$tid" "$ts_us" \
    >> "${_FLOX_PROFILE_DIR}/shell-trace.ndjson"
}

# Backward-compatible FLOX_ACTIVATE_TRACE stderr output
path0=$(echo "$PATH" | cut -d: -f1)
if realpath "$path0" | grep -q "^/nix/store/"; then
  echo "FLOX_ACTIVATE_TRACE:" "$*" 1>&2
else
  echo "FLOX_ACTIVATE_TRACE:" "$*" "path[0] = $path0" 1>&2
fi

# Write Chrome trace events based on START/END markers
_last_arg="${*: -1}"
_trace_name="${1:-unknown}"
case "$_last_arg" in
  START)
    _flox_profile_trace "B" "$_trace_name"
    ;;
  END)
    _flox_profile_trace "E" "$_trace_name"
    ;;
esac
