#! /usr/bin/env bash
# ============================================================================ #
#
# Bootstrap `flox' build system, regenerating any `autotools' resources.
#
# ---------------------------------------------------------------------------- #

set -eu;
set -o pipefail;


# ---------------------------------------------------------------------------- #

_as_script="${BASH_SOURCE[0]}";
_as_me="${_as_script##*/}";
_as_dir="${_as_script%/*}";

_version="0.1.0";

_usage_msg="USAGE: $_as_me [OPTIONS...]
Bootstraps \`flox' build system, regenerating any \`autotools' resources.
";

_help_msg="$_usage_msg
You should only need to run this if you are building from a \`git'
checkout and you've made changes to \`configure.ac'.
If you are building from a \`tarball' release or from CI, then you should not
run this script.

OPTIONS
  -h,--help         Print help message to STDOUT.
  -u,--usage        Print usage message to STDOUT.
  -v,--version      Print version information to STDOUT.

ENVIRONMENT
  ACLOCAL           Command used as \`aclocal' executable.
  AUTORECONF        Command used as \`autoreconf' executable.
  SED               Command used as \`sed' executable.
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
: "${ACLOCAL:=aclocal}";
: "${AUTORECONF:=autoreconf}";
: "${SED:=sed}";


# ---------------------------------------------------------------------------- #

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
      echo "$_as_me: Unexpected argument(s) '$*'" >&2;
      usage -f >&2;
      exit 1;
      ;;
  esac
  shift;
done


# ---------------------------------------------------------------------------- #

# Change directory to project root.
pushd "$_as_dir" >/dev/null||exit;


# ---------------------------------------------------------------------------- #

$ACLOCAL;
$AUTORECONF -iv;

# XXX: We patch `build-aux/m4/libtool.m4' instead of patching `configure' here,
#      but if you upgrade `libtool' you'll need to re-patch it with the
#      effect of:
$SED -i -e 's/\$RM \(\\"\$cfgfile\\";\)/$RM -f \1/'  \
        -e 's/\$RM \("\$cfgfile"\)/$RM -f \1/'      \
        ./configure;


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
