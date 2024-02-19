#! /usr/bin/env bash
# ============================================================================ #
#
# Detect changed source C++ files between two revisions.
# If no revisions are specified, `origin/main' and `HEAD' are used.
#
# ---------------------------------------------------------------------------- #

set -eu
set -o pipefail

# ---------------------------------------------------------------------------- #

_as_me="changed-sources.sh"

_version="0.1.0"

_usage_msg="USAGE: $_as_me [OPTIONS...] [OLD-SPEC [NEW-SPEC]]

Detect changed source C++ files between two revisions ( refspecs ).
"

_help_msg="${_usage_msg}\
If no revisions are specified, \`origin/main' and \`HEAD' are used.

Deleted files are not reported.
Added files are reported.

OPTIONS
  -a,--absolute     Print absolute paths.
  -h,--help         Print help message to STDOUT.
  -u,--usage        Print usage message to STDOUT.
  -v,--version      Print version information to STDOUT.

ENVIRONMENT
  GIT               Command used as \`git' executable.
  GREP              Command used as \`grep' executable.
  SED               Command used as \`sed' executable.
  SORT              Command used as \`sort' executable.
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
: "${GIT:=git}"
: "${GREP:=grep}"
: "${SED:=sed}"
: "${SORT:=sort}"

# ---------------------------------------------------------------------------- #

unset OLD_SPEC NEW_SPEC
ABSOLUTE=''

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
    -a | --absolute) ABSOLUTE=':' ;;
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
      if [[ -z "${OLD_SPEC:-}" ]]; then
        OLD_SPEC="$1"
      elif [[ -z "${NEW_SPEC:-}" ]]; then
        NEW_SPEC="$1"
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

# Set fallbacks
: "${OLD_SPEC:=origin/main}"
: "${NEW_SPEC:=HEAD}"

# ---------------------------------------------------------------------------- #

# Convert refspecs to commit hashes
_OLD_REV="$($GIT rev-parse --verify "$OLD_SPEC")"
_NEW_REV="$($GIT rev-parse --verify "$NEW_SPEC")"

# ---------------------------------------------------------------------------- #

# Get all modified files.
readarray -t _modified < <(
  {
    $GIT diff-tree --no-commit-id --name-only -r "$_OLD_REV" "$_NEW_REV" --;
    $GIT status --porcelain|$GREP '^\(A \| M\|??\) '|$SED 's/^.. //';
  }|$SORT -u;
)

# Filter down to C++ source files.
declare -a _sources
_sources=()
for mod in "${_modified[@]}"; do
  # Only keep files in `pkgdb/' subdirectories.
  case "$mod" in
    pkgdb/*) :; ;;
    *) continue; ;;
  esac

  case "$mod" in
    # Keep this list aligned with `.github/workflows/clang-tidy.yml'
    *.cpp | *.hpp | *.hxx | *.cxx | *.cc | *.hh | *.c | *.h | *.ipp) 
      _sources+=("$mod") ;
    ;;
    *) :; ;;
  esac
done

# ---------------------------------------------------------------------------- #

# If we got nothing don't print anything.
if [[ "${#_sources[@]}" -eq 0 ]]; then exit 0; fi

# ---------------------------------------------------------------------------- #

# Print results
if [[ -n "$ABSOLUTE" ]]; then
  _ROOT="$($GIT rev-parse --show-toplevel)/pkgdb"
  for src in "${_sources[@]}"; do echo "$_ROOT/$src"; done
else
  printf '%s\n' "${_sources[@]}"
fi

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
