#! /usr/bin/env bash
# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

set -eu;
set -o pipefail;


# ---------------------------------------------------------------------------- #

_as_me="config-matters.bash";

_version="0.1.0";

_usage_msg="USAGE: $_as_me [OPTIONS...] FILE

Detect whether \`nix/config.h' effects a header file.
";

_help_msg="$_usage_msg


OPTIONS
  -h,--help         Print help message to STDOUT.
  -u,--usage        Print usage message to STDOUT.
  -v,--version      Print version information to STDOUT.

ENVIRONMENT
  CPP               Command used as \`cpp' executable.
  DIFF              Command used as \`diff' executable.
  MKTEMP            Command used as \`mktemp' executable.
  nix_INCDIR        Directory containing \`nix/config.h'.
  nix_CFLAGS        Flags to pass to the C++ compiler, especially \`-I' flags.
                    This must NOT contain \`-include \$nix_INCDIR/nix/config.h'.
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
: "${MKTEMP:=mktemp}";
: "${DIFF:=diff}";
: "${CPP:=cpp}";


# ---------------------------------------------------------------------------- #

if [[ -z "${nix_INCDIR:-}" ]]; then
  echo "You must set the \`nix_INCDIR' environment variable" >&2;
  exit 1;
fi


# ---------------------------------------------------------------------------- #

if [[ -z "$nix_CFLAGS" ]]; then
  echo "$_as_me: You must set the \`nix_CFLAGS' environment variable" >&2;
  exit 1;
fi


# ---------------------------------------------------------------------------- #

declare -a tmp_files tmp_dirs;
tmp_files=();
tmp_dirs=();

mktmp_auto() {
  local _f;
  _f="$( $MKTEMP "$@"; )";
  case " $* " in
    *\ -d\ *|*\ --directory\ *) tmp_dirs+=( "$_f" ); ;;
    *)                          tmp_files+=( "$_f" ); ;;
  esac
  echo "$_f";
}


# ---------------------------------------------------------------------------- #

cleanup() {
  rm -f "${tmp_files[@]}";
  rm -rf "${tmp_dirs[@]}";
}

_es=0;
trap '_es="$?"; cleanup; exit "$_es";' HUP TERM INT QUIT EXIT;


# ---------------------------------------------------------------------------- #

declare -a _files;
_files=();
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
      if ! [[ -r "$1" ]]; then
        echo "$_as_me: Cannot read file: '$1'" >&2;
        exit 1;
      fi
      _files+=( "$1" );
    ;;
  esac
  shift;
done


# ---------------------------------------------------------------------------- #

if [[ "${#_files[@]}" -eq 0 ]]; then
  echo "$_as_me: No files specified" >&2;
  usage -f >&2;
  exit 1;
fi


# ---------------------------------------------------------------------------- #

# Aggregate `CPPFLAGS'.
declare -a _cppflags;
IFS=' ' read -r -a _cppflags <<< "$nix_CFLAGS";
_cppflags=( '-E' '-x' 'c++-header' '-isystem' "$nix_INCDIR" "${_cppflags[@]}" );


# ---------------------------------------------------------------------------- #

# configMatters FILE
# ------------------
# Returns 0 if the file is affected by `nix/config.h', 1 otherwise.
configMatters() {
  local _noConfig _withConfig;
  #shellcheck disable=SC2119
  _noConfig="$( mktmp_auto; )";
  #shellcheck disable=SC2119
  _withConfig="$( mktmp_auto; )";
  $CPP "${_cppflags[@]}" "${1?}" -o "$_noConfig";
  $CPP "${_cppflags[@]}" -include "$nix_INCDIR/nix/config.h" "$1"  \
       -o "$_withConfig";
  $DIFF -q "$_noConfig" "$_withConfig" > /dev/null 2>&1 && return 1;
}


# ---------------------------------------------------------------------------- #

for _f in "${_files[@]}"; do
  if configMatters "$_f"; then
    echo "$_f T";
  else
    echo "$_f F";
  fi
done

exit 0;


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
