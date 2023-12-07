#! /usr/bin/env bash
# ============================================================================ #
#
# Pre-processor for C++ source files.
#
# Convert my dank `/* ==== *' header blocks into JavaDoc style blocks.
#
# ---------------------------------------------------------------------------- #

set -eu;
set -o pipefail;


# ---------------------------------------------------------------------------- #

_as_me="fixup-headers.sh";

_version="0.1.0";

_usage_msg="USAGE: $_as_me [OPTIONS...] FILE
Pre-processor for a C/C++ source file.
";

_help_msg="$_usage_msg
Converts my dank \`/* ==== *' header blocks into JavaDoc style blocks.

OPTIONS
  -h,--help         Print help message to STDOUT.
  -u,--usage        Print usage message to STDOUT.
  -v,--version      Print version information to STDOUT.

ENVIRONMENT
  SED               Command used as \`grep' executable.
";


# ---------------------------------------------------------------------------- #

usage() {
  if [[ "${1:-}" = "-f" ]]; then
    echo "$_help_msg";
  else
    echo "$_usage_msg";
  fi
}


# ---------------------------------------------------------------------------- #

# @BEGIN_INJECT_UTILS@
: "${SED:=sed}";


# ---------------------------------------------------------------------------- #

unset TARGET;
while [[ "$#" -gt 0 ]]; do
  case "$1" in
    # Split short options such as `-abc' -> `-a -b -c'
    -[^-]?*)
      _arg="$1";
      declare -a _args;
      _args=();
      shift;
      _i=1;
      while [[ "$_i" -lt "${#_arg}" ]]; do
        _args+=( "-${_arg:$_i:1}" );
        _i="$(( _i + 1 ))";
      done
      set -- "${_args[@]}" "$@";
      unset _arg _args _i;
      continue;
    ;;
    --*=*)
      _arg="$1";
      shift;
      set -- "${_arg%%=*}" "${_arg#*=}" "$@";
      unset _arg;
      continue;
    ;;
    -u|--usage)    usage;    exit 0; ;;
    -h|--help)     usage -f; exit 0; ;;
    -v|--version)  echo "$_version"; exit 0; ;;
    --) shift; break; ;;
    -?|--*)
      echo "$_as_me: Unrecognized option: '$1'" >&2;
      usage -f >&2;
      exit 1;
    ;;
    *)
      if [[ -z "${TARGET:-}" ]]; then
        TARGET="$1";
      else
        echo "$_as_me: Unexpected argument(s) '$*'" >&2;
        usage -f >&2;
        exit 1;
      fi
    ;;
  esac
  shift;
done


# ---------------------------------------------------------------------------- #

if [[ -z "${TARGET:-}" ]]; then
  echo "$_as_me: You must provide a path to a target file" >&2;
  exit 1;
fi


# ---------------------------------------------------------------------------- #

$SED -e "s,^/\* =====* \*\$,/**," -e 's,^ \* -----* \*/$, */,' "$TARGET";


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
