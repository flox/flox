#!/bin/bash

set -euo pipefail

# validate-build: following a "successful" build, perform a series of checks to
# validate the build and provide the user with hints as to how to address any
# issues found.

_basename="@coreutils@/bin/basename"
_env="@coreutils@/bin/env"
_jq="@jq@/bin/jq"
_nix="@nix@/bin/nix"
_nix_store="@nix@/bin/nix-store"
_getopt="@getopt@/bin/getopt"
_realpath="@coreutils@/bin/realpath"

# Parse command-line arguments.
me="$($_basename "$0")"
OPTIONS="dhs:x:"
LONGOPTS="debug,help,build-env:,develop-env:,pname:,system:,extra-requisites:"
USAGE="Usage: $me [(-d|--debug)] [(-h|--help)] \
  --pname <pname>       package name
  --system <system>     Nix system type
  --build-env <path>    path to the build environment
  --develop-env <path>  path to the develop environment
  (-x|--extra-requisite) <path>
                        extra requisite to allow in the build
  <output>              path to the built package"
if ! PARSED="$("$_getopt" --options="$OPTIONS" --longoptions="$LONGOPTS" --name "$0" -- "$@")"; then
    echo "Failed to parse options."
    exit 1
fi

# Use eval to remove quotes and replace them with spaces.
eval set -- "$PARSED"

# Set default values for options.
declare _build_env=""
declare _develop_env=""
declare _output=""
declare _pname=""
declare _system=""
declare -a _extra_requisites=()
while true; do
    case "$1" in
        -d|--debug)
            set -x
            shift
            ;;
        -h|--help)
            echo "$USAGE" >&2
            exit 0
            ;;
        --build-env)
            shift
            if [ -z "${1:-}" ]; then
                echo "Option --build-env requires a path as an argument." >&2
                echo "$USAGE" >&2
                exit 1
            fi
            _build_env="$1"
            shift
            ;;
        --develop-env)
            shift
            if [ -z "${1:-}" ]; then
                echo "Option --develop-env requires a path as an argument." >&2
                echo "$USAGE" >&2
                exit 1
            fi
            _develop_env="$1"
            shift
            ;;
        --pname)
            shift
            if [ -z "${1:-}" ]; then
                echo "Option --pname requires a package name as an argument." >&2
                echo "$USAGE" >&2
                exit 1
            fi
            _pname="$1"
            shift
            ;;
        -s|--system)
            shift
            if [ -z "${1:-}" ]; then
                echo "Option --system requires a valid nix system type as an argument." >&2
                echo "$USAGE" >&2
                exit 1
            fi
            _system="$1"
            shift
            ;;
         -x|--extra-requisites)
            shift
            if [ -z "${1:-}" ]; then
                echo "Option --extra-requisites requires a path as an argument." >&2
                echo "$USAGE" >&2
                exit 1
            fi
            _extra_requisites+=("$1")
            shift
            ;;
        --)
            shift
            break
            ;;
        -*)
            echo "Invalid option: $1" >&2
            echo "$USAGE" >&2
            exit 1
            ;;
    esac
done

# Verify that all required arguments have been provided and are valid.

if [ -z "$_build_env" ]; then
    echo "ERROR: build environment path not specified." >&2
    echo "$USAGE" >&2
    exit 1
fi
if [ ! -d "$_build_env" ]; then
    echo "ERROR: build environment path '$_build_env' does not exist." >&2
    exit 1
fi
# Resolve any symbolic links in the build environment path.
_build_env="$("$_realpath" "$_build_env")"

if [ -z "$_develop_env" ]; then
    echo "ERROR: develop environment path not specified." >&2
    echo "$USAGE" >&2
    exit 1
fi
if [ ! -d "$_develop_env" ]; then
    echo "ERROR: develop environment path '$_develop_env' does not exist." >&2
    exit 1
fi
# Resolve any symbolic links in the develop environment path.
_develop_env="$("$_realpath" "$_develop_env")"

# Verify that the extra requisites are valid directories.
for req in "${_extra_requisites[@]}"; do
    if [ ! -e "$req" ]; then
        echo "ERROR: extra requisite path '$req' does not exist." >&2
        exit 1
    fi
done

# Verify that the package name is provided and valid.
if [ -z "$_pname" ]; then
    echo "ERROR: package name not specified." >&2
    echo "$USAGE" >&2
    exit 1
fi

# Verify that the system type is provided and valid.
case "$_system" in
    x86_64-linux|aarch64-linux|x86_64-darwin|aarch64-darwin)
        # Valid system type, do nothing.
        ;;
    *)
        echo "ERROR: invalid system type '$_system'." >&2
        echo "Valid system types are: x86_64-linux, aarch64-linux, x86_64-darwin, aarch64-darwin." >&2
        exit 1
        ;;
esac

# Check that the output package path is provided.
if [ $# -ne 1 ]; then
    echo "ERROR: expected exactly one output package path." >&2
    echo "$USAGE" >&2
    exit 1
fi
# Check that the output package path is valid.
if [ ! -e "$1" ]; then
    echo "ERROR: output package path '$1' does not exist." >&2
    exit 1
fi
# Resolve any symbolic links in the output package path.
_output="$("$_realpath" "$1")"

# Functions

# query_requisites <out_array_name> <nix-store-path>
#     populates associative array named by $1 with keys = each requisite of $2
function query_requisites() {
    local -n reqs="$1"; shift

    # read all lines into a temp array, then index into the assoc-array
    local -a tmp
    readarray -t tmp < <("$_nix_store" --query --requisites "$@")
    for dep in "${tmp[@]}"; do
        # shellcheck disable=SC2034
        reqs["$dep"]=1
    done
}

# diff_assoc_keys <output_array_name> <allowed_array_name> <result_array_name>
#   result = keys(out) \ keys(allowed)
function diff_assoc_keys {
    local -n out=$1
    local -n allowed=$2
    local -n result=$3

    result=()  # reset the result array

    for key in "${!out[@]}"; do
        # if allowed[$key] is unset, include it
        if [[ -z "${allowed[$key]+_}" ]]; then
            result+=("$key")
        fi
    done
}

# verify_requistites(): verify that the build contains only the packages
#         found in the build wrapper's closure and/or any "extra" requisites.
#         Uses global variables _build_env, _develop_env, _output, and
#         _extra_requisites.
function verify_requisites() {
    local -A _output_requisites=()
    local -A _allowed_requisites=()

    # Populate requisites for the build output.
    query_requisites _output_requisites "$_output"

    # Populate requisites for the build environment.
    # BUG: $_build_env/requisites.txt should be all we need here, but it is
    #      missing libcxx on Darwin??? Repeat the hard/expensive way ...
    query_requisites _allowed_requisites "$_build_env"

    # Populate requisites for the "extra" requisites.
    query_requisites _allowed_requisites "${_extra_requisites[@]}"

    # Also allow package to refer to itself.
    _allowed_requisites["$_output"]=1

    # Calculate set of extra packages found in the output that are not
    # present in the set of allowed requisites.
    local -a _extra_packages

    # populate _extra_packages with keys in _output_requisites not in _allowed_requisites
    diff_assoc_keys _output_requisites _allowed_requisites _extra_packages

    # If there are any extra packages, report them.
    if [[ ${#_extra_packages[@]} -gt 0 ]]; then
        # Display the first 3 extra packages.
        local _count=${#_extra_packages[@]}
        local _space
        _space="$(printf '%*s' "${#_count}" "")"
        cat <<EOF
❌ ERROR: Unexpected dependencies found in package '$_pname':

1. Remove any unneeded references (e.g. debug symbols) from your build.
2. If you’re using package groups, move these packages into the 'toplevel' group.
3. If you’re using 'runtime-packages', make sure each package is listed both in
   'runtime-packages' and in the 'toplevel' group.

$_count packages found in $_output
$_space      not found in $_build_env

EOF
        if [[ $_count -gt 3 ]]; then
            echo "Displaying first 3 only:"
            _count=3
        fi

        for pkg in "${_extra_packages[@]:0:$_count}"; do
            "$_nix" --extra-experimental-features nix-command \
                why-depends --precise "$_output" "$pkg" || true
            echo
        done

        exit 1
    fi
}

# find_packages_in_env <out_array_name> <pkg> <env>
#  populates associative array named by $1 with packages in <env> that
#  contain <pkg> in their closure.
function find_packages_in_env {
    local -n out="$1"; shift
    local pkg="$1"; shift
    local env="$1"; shift

    # Iterate over the closures of packages installed to the environment
    # looking for the given package, and if found then add the package
    # name to the hints associative array named by $out.
    while IFS=: read -r install_id group output_path; do
        # Calculate the closure for the provided output_path, and
        # if it contains the given package, then add the install_id
        # to the hints associative array named by $out.
        local -A output_path_closure=()
        query_requisites output_path_closure "$output_path"
        if [[ -n "${output_path_closure[$pkg]+_}" ]]; then
            out["$install_id"]="$group"
        fi
    done < <(
        $_jq -r " \
            .packages | \
            map(select(.system==\"$_system\"))[] | \
            .install_id as \$id | \
            .group as \$group | \
            .outputs | to_entries | \
            map(\"\(\$id):\(\$group):\(.value)\")[] \
        " "$env/manifest.lock"
    )
}

# report_missing_files <output_dir> <build_env> <develop_env>
#  - scans all files under <output_dir> for literal references to <build_env> paths
#  - for each referenced path that does NOT exist in <build_env>:
#      • prints the missing path
#      • prints the file under <output_dir> that made the reference
#      • checks if an equivalent file lives under <develop_env>, and if so:
#          – prints its realpath
#          – hints to add its package to runtime-packages
function report_missing_files {
    local output_dir="$1"
    local build_env="$2"
    local develop_env="$3"
    local -i rc=0

    # Recursively grep for any substring starting with $build_env up to the next whitespace
    # -R : recursive
    # -H : print filename
    # -o : print only the matched portion
    # --binary-files=text : treat binaries as text
    # -e : pattern
    while IFS=: read -r src_file ref_path; do
        # If the path really doesn't exist in the build env...
        if [[ ! -e $ref_path ]]; then
            local -a hints=()
            # Return nonzero.
            rc=1

            # compute the same relative suffix under develop_env
            local rel="${ref_path#"$build_env"}"
            local basename="${ref_path##*/}"
            local dev_path="$develop_env$rel"

            if [[ -e "$dev_path" ]]; then
                # realpath gives the absolute, canonical path in the store
                local real
                real="$(realpath "$dev_path")"
                # If we take the first 3 path elements we get the storepath
                # of the package that provides the file.
                local storepath
                storepath="${real%/*/*}"
                # Calculate hints by searching for realpath in closure of
                # packages installed to develop environment.
                local -A _hint_pkgs=()
                find_packages_in_env _hint_pkgs "$storepath" "$develop_env"

                # If we did find the file's package in any of the installed
                # package closures then report those hints.
                if [[ ${#_hint_pkgs[@]} -gt 0 ]]; then
                    local -a hints=()
                    for i in "${!_hint_pkgs[@]}"; do
                        case "${_hint_pkgs[$i]}" in
                            toplevel)
                                # If the package is in the toplevel group, then
                                # suggest adding it to runtime-packages.
                                hints+=("consider adding package '$i' to 'runtime-packages'")
                                ;;
                            *)
                                # Otherwise, suggest moving it to the toplevel group.
                                hints+=("consider moving package '$i' to 'toplevel' pkg-group")
                                ;;
                        esac
                    done
                else
                    hints+=("report bug to Flox: '$dev_path' not referenced from manifest")
                fi
            else
                hints+=("check your build script and project files for any mention of the '$basename' string")
            fi
            printf "❌ ERROR: Nonexistent path reference to '%s' found in package '%s':\n" \
                "\$FLOX_ENV$rel" "$_pname"
            for i in "${hints[@]}"; do
                echo "    Hint: $i"
            done
            printf "%s\n%s\n\n" \
                "Path referenced by: $src_file" \
                "  Nonexistent path: $ref_path"
        fi
    # Note: we assume filenames and ref_paths contain no colons.
    done < <(
        grep --binary-files=text -RHo -e "${build_env}[^:\"'[:cntrl:][:space:]]*" -- "$output_dir"
    )

    return $rc
}

#
# main()
#

# Verify the requisites.
verify_requisites >&2

# Comb through the build looking for references to the build wrapper
# environment and ensure that each file referenced actually exists.
# For any files that are missing, report the package from which they
# come from in the "develop" environment, and hint that users may
# want to add these files to the "runtime-packages" attribute.
report_missing_files "$_output" "$_build_env" "$_develop_env" >&2

exit 0
