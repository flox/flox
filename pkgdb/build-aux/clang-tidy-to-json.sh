#! /usr/bin/env bash
# ============================================================================ #
#
# Split `clang-tidy' output lines to JSON for easier processing.
#
# ---------------------------------------------------------------------------- #

set -eu
set -o pipefail

# ---------------------------------------------------------------------------- #

_as_me="clang-tidy-to-json.sh"

_version="0.1.0"

_usage_msg="USAGE: $_as_me [OPTIONS...] [FILE=STDIN]

Split \`clang-tidy' output lines to JSON for easier processing.
"

_help_msg="${_usage_msg}\

OPTIONS
  -h,--help         Print help message to STDOUT.
  -u,--usage        Print usage message to STDOUT.
  -v,--version      Print version information to STDOUT.

ENVIRONMENT
  JQ                Command used as \`jq' executable.
"

# ---------------------------------------------------------------------------- #

usage() {
  if [[ "${1:-}" = "-f" ]]; then
    echo "$_help_msg"
  else
    echo "$_usage_msg"
  fi
}

# ---------------------------------------------------------------------------- #

# @BEGIN_INJECT_UTILS@
: "${JQ:=jq}"

# ---------------------------------------------------------------------------- #

unset _TARGET

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    # Split short options such as `-abc' -> `-a -b -c'
    -[^-]?*)
      _arg="$1"
      declare -a _args
      _args=()
      shift
      _i=1
      while [[ "$_i" -lt "${#_arg}" ]]; do
        _args+=("-${_arg:$_i:1}")
        _i="$((_i + 1))"
      done
      set -- "${_args[@]}" "$@"
      unset _arg _args _i
      continue
      ;;
    --*=*)
      _arg="$1"
      shift
      set -- "${_arg%%=*}" "${_arg#*=}" "$@"
      unset _arg
      continue
      ;;
    -u | --usage)
      usage
      exit 0
      ;;
    -h | --help)
      usage -f
      exit 0
      ;;
    -v | --version)
      echo "$_version"
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -? | --*)
      echo "$_as_me: Unrecognized option: '$1'" >&2
      usage -f >&2
      exit 1
      ;;
    *)
      if [[ -z "${_TARGET:-}" ]]; then
        _TARGET="$1"
      else
        echo "$_as_me: Unexpected argument(s) '$*'" >&2
        usage -f >&2
        exit 1
      fi
      ;;
  esac
  shift
done

# ---------------------------------------------------------------------------- #

_FILE=''
_LINE=''
_COL=''
_KIND=''
_TITLE=''
declare -a _body

parseHeader() {
  local _full="${1?You must provide a line to parse}"
  _FILE="${_full%%:*}"
  local _rest="${_full#*:}"
  _LINE="${_rest%%:*}"
  _rest="${_rest#*:}"
  _COL="${_rest%%:*}"
  _rest="${_rest#*: }"
  _KIND="${_rest%%:*}" # warning|error
  _TITLE="${_rest#*: }"
}

# ---------------------------------------------------------------------------- #

printCurrent() {
  echo "{
  \"file\": \"$_FILE\",
  \"line\": $_LINE,
  \"column\": $_COL,
  \"kind\": \"$_KIND\",
  \"title\": \"${_TITLE//\"/\\\"}\",
  \"body\": ["
  local _first=:
  for _line in "${_body[@]}"; do
    if [[ "$_first" = ':' ]]; then
      _first=
      printf '    "'
    else
      printf '  , "'
    fi
    echo "${_line//\"/\\\"}\""
  done
  echo "  ]"
  echo '}'
}

# ---------------------------------------------------------------------------- #

_body=()

shopt -s extglob
while IFS='' read -r line; do
  case "$line" in
    +([^:]):+([[:digit:]]):+([[:digit:]]):\ +(warning|error):\ *)
      if [[ -n "${_KIND:-}" ]]; then printCurrent | $JQ -cM; fi
      _body=()
      parseHeader "$line"
      ;;
    *) _body+=("$line") ;;
  esac
done < "${_TARGET:=/dev/stdin}"

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
