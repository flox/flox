#! /usr/bin/env bash
# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

set -eu;
set -o pipefail;


# ---------------------------------------------------------------------------- #

_as_me="manifest.sh";

_version="0.1.0";

_usage_msg="USAGE: $_as_me [OPTIONS...] SUBCOMMAND
";

_help_msg="$_usage_msg
SUBCOMMANDS
  floxpkgsToFlakeref
  flakerefToFloxpkg
  floxpkgsToPosition
  flakerefToPosition
  storepathToPosition
  positionToFloxpkgs
  listEnvironment
  convert007to008
  listFlakesInEnvironment
  listStorePaths
  flakerefToNixEditorArgs
  floxpkgToNixEditorArgs
  positionToCatalogPath
  dump

OPTIONS
  -h,--help         Print help message to STDOUT.
  -u,--usage        Print usage message to STDOUT.
  -v,--version      Print version information to STDOUT.

ENVIRONMENT
  NIX               Command used as \`nix' executable.
  JQ                Command used as \`jq' executable.
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
: "${NIX:=nix}";
: "${JQ:=jq}";


# ---------------------------------------------------------------------------- #

unset _subcmd;

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

    floxpkgToFlakeref) _subcmd="$1"; shift; break; ;;

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

_emptyManifest='[{"elements":[],"version":2}]';


# ============================================================================ #
#
# Functions which convert between flakeref and floxpkg tuple elements.
#
# floxpkg: <stability>.<channel>.<pkgname> (fully-qualified)
# flake:<channel>#evalCatalog.<system>.<stability>.<pkgname>
#
# Sample element:
# {
#   "active": true,
#   "attrPath": "evalCatalog.$system.stable.vim",
#   "originalUrl": "flake:nixpkgs-flox",
#   "storePaths": [
#     "/nix/store/ivwgm9bdsvhnx8y7ac169cx2z82rwcla-vim-8.2.4350"
#   ],
#   "url": "github:flox/nixpkgs-flox/ef23087ad88d59f0c0bc0f05de65577009c0c676",
#   "position": 3
# }
#
#
# ---------------------------------------------------------------------------- #

# For `FLOX_SYSTEM' to be a global variable, but don't export.
: "${FLOX_SYSTEM:=}";
getFloxSystem() {
  # Set `FLOX_SYSTEM' if it is unset/empty.
  : "${FLOX_SYSTEM:=$(
    $NIX --experimental-features 'nix-command flakes' eval --raw --impure  \
         --expr builtins.currentSystem;
  )}";
  echo "$FLOX_SYSTEM";
}


# ---------------------------------------------------------------------------- #

# floxpkgToFlakeref <STABILITY>.<CATALOG>.<ATTR>[.<ATTR>]...
#   =>  flake:<CATALOG>#evalCatalog.<SYSTEM>.<STABILITY>.<ATTR>[.<ATTR>]...
floxpkgToFlakeref() {
  local _fpr="$1";
  local _stability="${_fpr%%.*}";
  local _catalog="${_fpr#*.}";     # remove stability
  _catalog="${_catalog%%.*}";
  local _attrPath="${_fpr#*.*.}";
  if [[ "$_stability.$_catalog.$_attrPath" != "$_fpr" ]]; then
    echo "$_as_me: Failed to split floxpkg reference: '$_fpr'." >&2;
    exit 1;
  fi
  echo "flake:$_catalog#evalCatalog.$( getFloxSystem; ).$_stability.$_attrPath";
}


# ---------------------------------------------------------------------------- #

$_subcmd "$@";
exit "$?";


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
