#! /usr/bin/env bash
# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

# Push current options to be restored later.
_pkgdb_OLD_OPTS="$( set +o; )";

set -eu;
set -o pipefail;


# ---------------------------------------------------------------------------- #

_as_me="${BASH_SOURCE[0]##*/}";
_as_dir="${BASH_SOURCE[0]%/*}";


# ---------------------------------------------------------------------------- #

# @BEGIN_INJECT_UTILS@
: "${PKGDB:=pkgdb}";
: "${REALPATH:=realpath}";
: "${MKTEMP:=mktemp}";
: "${MKDIR:=mkdir}";
: "${RM:=rm}";
# @END_INJECT_UTILS@


# ---------------------------------------------------------------------------- #

MKTMP_OUTPUT="";
export _CLEANUP_TMPFILES="";
export _CLEANUP_TMPDIRS="";

mktmp_auto() {
  local _f;
  _f="$( $MKTEMP "$@"; )";
  case " $* " in
    *\ -d\ *|*\ --directory\ *)
      _CLEANUP_TMPDIRS="${_CLEANUP_TMPDIRS:+$_CLEANUP_TMPDIRS:}$_f";
      export _CLEANUP_TMPDIRS;
    ;;
    *)
      _CLEANUP_TMPFILES="${_CLEANUP_TMPFILES:+$_CLEANUP_TMPFILES:}$_f";
      export _CLEANUP_TMPFILES;
    ;;
  esac
  MKTMP_OUTPUT="$_f";
}


# ---------------------------------------------------------------------------- #

cleanup() {
  echo "$_as_me: Cleaning up..." >&2;
  declare -a _targets;
  IFS=":" read -ra _targets < <( echo "$_CLEANUP_TMPFILES"; );
  if [[ -n "${_targets[*]}" ]]; then $RM -f "${_targets[@]}"; fi;
  IFS=":" read -ra _targets < <( echo "$_CLEANUP_TMPDIRS"; );
  if [[ -n "${_targets[*]}" ]]; then $RM -rf "${_targets[@]}"; fi;
}

_es=0;
trap '_es="$?"; cleanup; exit "$_es";' HUP TERM INT QUIT EXIT;


# ---------------------------------------------------------------------------- #

# Create temporary for the package databases.
mktmp_auto -d;
_PKGDB_TMP="$MKTMP_OUTPUT";
$MKDIR -p "$_PKGDB_TMP";

export XDG_CACHE_HOME="$_PKGDB_TMP/cache";

echo "$_as_me: Created temporary dir: $_PKGDB_TMP" >&2;


# ---------------------------------------------------------------------------- #

echo "$_as_me: Scraping Nixpkgs..." >&2;

$PKGDB scrape "github:NixOS/nixpkgs/release-23.11" legacyPackages x86_64-linux &
sleep 8s;

echo "$_as_me: Terminating \`pkgdb'..." >&2;
kill -s KILL %1;


# ---------------------------------------------------------------------------- #

ls -lR "$_PKGDB_TMP/cache" >&2;
echo '' >&2;

cp -r "$_PKGDB_TMP/cache/flox" "$PWD/flox-cache";


# ---------------------------------------------------------------------------- #

$PKGDB get path "github:NixOS/nixpkgs/release-23.11" 1;


# ---------------------------------------------------------------------------- #

# Restore original options.
#eval "$_pkgdb_OLD_OPTS";
#unset _pkgdb_OLD_OPTS;


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
