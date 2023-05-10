#
# Utility functions.
#

# Color highlighting variables.
ESC="\x1b["

# flox color palette.
#201e7b, nearest named: midnight blue(HTML104), dark slate blue(HTML122)*
DARKBLUE="32;30;123"
DARKBLUE256=17 # NavyBlue, by eye
#58569c, nearest named: dark slate blue(HTML122), slate blue(HTML123)*
LIGHTBLUE="88;86;156"
LIGHTBLUE256=61 # SlateBlue3
#ffceac, nearest named: peach puff(HTML32)*, navajo white(HTML40)
LIGHTPEACH="255;206;172"
LIGHTPEACH256=223 # NavajoWhite1
#ffb990, nearest named: dark salmon(HTML11), light salmon(HTML9)*
DARKPEACH="255;185;144"
DARKPEACH256=216 # LightSalmon1
# 256-color terminal escape sequences.
floxDarkBlue="${ESC}38;5;${DARKBLUE256}m"
floxLightBlue="${ESC}38;5;${LIGHTBLUE256}m"
floxDarkPeach="${ESC}38;5;${DARKPEACH256}m"
floxLightPeach="${ESC}38;5;${LIGHTPEACH256}m"

# Standard 16-color escape sequences.
colorBlack="${ESC}0;30m"
colorDarkGray="${ESC}1;30m"
colorRed="${ESC}0;31m"
colorLightRed="${ESC}1;31m"
colorGreen="${ESC}0;32m"
colorLightGreen="${ESC}1;32m"
colorOrange="${ESC}0;33m"
colorYellow="${ESC}1;33m"
colorBlue="${ESC}0;34m"
colorLightBlue="${ESC}1;34m"
colorPurple="${ESC}0;35m"
colorLightPurple="${ESC}1;35m"
colorCyan="${ESC}0;36m"
colorLightCyan="${ESC}1;36m"
colorLightGray="${ESC}0;37m"
colorWhite="${ESC}1;37m"

# Simple font effects.
colorReset="${ESC}0m"
colorBold="${ESC}1m"
colorFaint="${ESC}2m"
colorItalic="${ESC}3m"
colorUnderline="${ESC}4m"
colorSlowBlink="${ESC}5m"
colorRapidBlink="${ESC}6m"
colorReverseVideo="${ESC}7m"

# Set gum color palette.
export GUM_SPIN_SPINNER_FOREGROUND=$DARKPEACH256
export GUM_CHOOSE_CURSOR_FOREGROUND="$DARKPEACH256"
export GUM_CHOOSE_PROMPT_FOREGROUND="$LIGHTBLUE256"
export GUM_CHOOSE_SELECTED_CURSOR_FOREGROUND="$DARKPEACH256"
export GUM_CHOOSE_SELECTED_PROMPT_FOREGROUND="$LIGHTBLUE256"

export GUM_FILTER_INDICATOR_FOREGROUND="$LIGHTBLUE256"
export GUM_FILTER_MATCH_FOREGROUND="$DARKPEACH256"
export GUM_FILTER_PROMPT_FOREGROUND="$DARKBLUE256"

export GUM_INPUT_CURSOR_FOREGROUND="$DARKPEACH256"
export GUM_INPUT_PROMPT_FOREGROUND="$DARKPEACH256"

export GUM_CONFIRM_SELECTED_FOREGROUND="$DARKBLUE256"
export GUM_CONFIRM_SELECTED_BACKGROUND="$LIGHTPEACH256"
export GUM_CONFIRM_PROMPT_FOREGROUND="$DARKPEACH256"

# Set flox prompt colors.
export FLOX_PROMPT_COLOR_1=${FLOX_PROMPT_COLOR_1:-$LIGHTBLUE256}
export FLOX_PROMPT_COLOR_2=${FLOX_PROMPT_COLOR_2:-$DARKPEACH256}

function pprint() {
	# Redirect the output of set -x to /dev/null
	exec 9>/dev/null
	local BASH_XTRACEFD=9
	# Step through args and encase with single-quotes those which need it.
	local space=""
	for i in "$@"; do
		if [ -z "$i" ]; then
			# empty arg
			echo -e -n "$space''"
		elif [[ "$i" =~ ^\'.*\'$ ]]; then
			# already quoted
			echo -e -n "$space$i"
		elif [[ "$i" =~ ^\".*\"$ ]]; then
			# already quoted(?)
			echo -e -n "$space$i"
		elif [[ "$i" =~ ([ !?*&()|]) ]]; then
			echo -e -n "$space'$i'"
		else
			echo -e -n "$space$i"
		fi
		space=" "
	done
	echo ""
}

#
# trace( <args> )
#
# Utility function which prints to STDERR a colorized call stack
# along with the supplied args.
filecolor=$colorBold
funccolor=$colorCyan
argscolor=$floxLightPeach
function trace() {
	# Redirect the output of set -x to /dev/null
	exec 9>/dev/null
	local BASH_XTRACEFD=9
	[ $debug -gt 0 ] || return 0
	echo -e "trace:${filecolor}${BASH_SOURCE[2]}:${BASH_LINENO[1]}${colorReset} ${funccolor}${FUNCNAME[1]}${colorReset}( ${argscolor}"$(pprint "$@")"${colorReset} )" 1>&2
}

# Track exported environment variables for use in verbose output.
declare -A exported_variables
function hash_commands() {
	trace "$@"
	set -h # explicitly enable hashing
	local PATH=@@FLOXPATH@@:$PATH
	for i in $@; do
		_i=${i//-/_} # Pesky utilities containing dashes require rewrite.
		hash $i # Dies with useful/precise error on failure when not found.
		declare -g _$_i=$(type -P $i)

		# Define $invoke_<name> variables for those invocations we'd
		# like to wrap with the invoke() subroutine.
		declare -g invoke_$_i="invoke $(type -P $i)"

		# Some commands require certain environment variables to work properly.
		# Make note of them here for displaying verbose output in invoke().
		case $i in
		nix | nix-store)
			exported_variables[$(type -P $i)]="NIX_REMOTE NIX_SSL_CERT_FILE NIX_USER_CONF_FILES GIT_CONFIG_SYSTEM" ;;
		*) ;;
		esac
	done
}

# Before doing anything take inventory of all commands required by the script.
# Note that we specifically avoid modifying the PATH environment variable to
# avoid leaking Nix paths into the commands we invoke.
# TODO replace each use of $_cut and $_tr with shell equivalents.
hash_commands \
	ansifilter awk basename bash cat chmod cmp column cp curl cut dasel date dirname \
	getent gh git grep gum id jq ln man mkdir mktemp mv nix nix-editor nix-store \
	pwd readlink realpath rm rmdir sed sh sleep sort stat tail tar tee \
	touch tr uname uuid xargs zgrep

# Return full path of first command available in PATH.
#
# Usage: first_in_PATH foo bar baz
function first_in_PATH() {
	trace "$@"
	set -h # explicitly enable hashing
	local PATH=@@FLOXPATH@@:$PATH
	for i in $@; do
		if hash $i 2>/dev/null; then
			echo $(type -P $i)
			return
		fi
	done
}

bestAvailableEditor=$(first_in_PATH vim vi nano emacs ed)
editorCommand=${EDITOR:-${VISUAL:-${bestAvailableEditor:-vi}}}

# Short name for this script, derived from $0.
me="${0##*/}"
mespaces=$(echo $me | $_tr '[a-z]' ' ')
medashes=$(echo $me | $_tr '[a-z]' '-')

# info() prints to STDERR
function info() {
	trace "$@"
	[ ${#@} -eq 0 ] || echo "$@" 1>&2
}

# warn() prints to STDERR in bold color
function warn() {
	trace "$@"
	[ ${#@} -eq 0 ] || echo -e "${colorBold}${@}${colorReset}" 1>&2
}

# verboseExec() uses pprint() to safely print exec() calls to STDERR
function verboseExec() {
	trace "$@"
	[ $verbose -eq 0 ] || warn $(pprint "+" "$@")
	exec "$@"
}

# error() prints spaces around the arguments, prints to STDERR in
# bold color and then exits nonzero (unless in interactive shell).
function error() {
	trace "$@"
	info "" # Add space before printing error.
	[ ${#@} -eq 0 ] || warn "ERROR: $@"
	info "" # Add space before appending output.
	# Relay any STDIN out to STDERR.
	$_cat 1>&2
	# Don't exit from interactive shells (for debugging).
	case "$-" in
	*i*) : ;;
	*) exit 1 ;;
	esac
}

# Add to $tmpFiles and $tmpDirs to ensure they're cleaned up upon exit.
# Be careful adding to $tmpDirs as it is recursively removed.
declare -a tmpFiles=()
declare -a tmpDirs=()
function cleanup() {
	# Keep temp files if debugging.
	if [ $debug -eq 0 ]; then
		if [ ${#tmpFiles[@]} -gt 0 ]; then
			$invoke_rm -f "${tmpFiles[@]}"
		fi
		if [ ${#tmpDirs[@]} -gt 0 ]; then
			for i in "${tmpDirs[@]}"; do
				if [[ $i =~ ^/tmp || $i =~ ^${TMPDIR} ]]; then
					$invoke_rm -rf "$i"
				else
					warn "cowardly refusing to recursively remove '$i'"
				fi
			done
		fi
	fi
}
trap cleanup EXIT

function mkTempFile() {
	local tmpFile
	tmpFile=$($_mktemp)
	tmpFiles+=($tmpFile)
	echo $tmpFile
}

function mkTempDir() {
	local tmpDir
	tmpDir=$($_mktemp -d)
	tmpDirs+=($tmpDir)
	echo $tmpDir
}

declare -A _usage
declare -A _usage_options
declare -a _general_commands
declare -a _development_commands
declare -a _environment_commands

# Pattered after output of `git -h`.
function usage() {
	trace "$@"
	$_cat <<EOF 1>&2
usage:
    $me [(-h|--help)] [--version] [--prefix]

general commands:
    $me [(-v|--verbose)] [--debug] <command> [<args>]
    $medashes
EOF

	for _command in "${_general_commands[@]}"; do
		if [ ${_usage_options["$_command"]+_} ]; then
			echo "    $me $_command ${_usage_options[$_command]}"
			echo "         - ${_usage[$_command]}"
		else
			echo "    $me $_command - ${_usage[$_command]}"
		fi
	done 1>&2
	echo "" 1>&2

	$_cat <<EOF 1>&2
environment commands:
    $me <command> [(-e|--environment) <env>] [<args>]
    $medashes
EOF

	for _command in "${_environment_commands[@]}"; do
		if [ ${_usage_options["$_command"]+_} ]; then
			echo "    $me $_command ${_usage_options[$_command]}"
			echo "         - ${_usage[$_command]}"
		else
			echo "    $me $_command - ${_usage[$_command]}"
		fi
	done 1>&2
	echo "" 1>&2

	$_cat <<EOF 1>&2
development commands:
    $me [--stability (stable|staging|unstable)] \\
    $mespaces [(-d|--date) <date_string>] <command> [<args>]
    $medashes
EOF

	for _command in "${_development_commands[@]}"; do
		if [ ${_usage_options["$_command"]+_} ]; then
			echo "    $me $_command ${_usage_options[$_command]}"
			echo "         - ${_usage[$_command]}"
		else
			echo "    $me $_command - ${_usage[$_command]}"
		fi
	done 1>&2
	echo "" 1>&2

}

#
# invoke(${cmd_and_args[@]})
#
# Helper function to print invocation to terminal when
# running with verbose flag.
#
declare -i minverbosity=1
function invoke() {
	# Redirect the output of set -x to /dev/null
	exec 9>/dev/null
	local BASH_XTRACEFD=9
	trace "$@"
	local vars=()
	if [ $verbose -ge $minverbosity ]; then
		for i in ${exported_variables[$1]}; do
			vars+=($(eval "echo $i=\${$i}"))
		done
		pprint "+$colorBold" "${vars[@]}" "$@" "$colorReset" 1>&2
	fi
	"$@"
}

#
# manifest(manifest,command,[args])
#
# Accessor method for jq-based manifest library functions.
# N.B. requires $manifest variable pointing to manifest.json file.
#
function manifest() {
	trace "$@"
	local manifest="$1"; shift
	# jq args:
	#   -n \                        # null input
	#   -e \                        # exit nonzero on errors
	#   -r \                        # raw output (i.e. don't add quotes)
	#   -f $_lib/manifest.jq \      # the manifest processing library
	#   --arg system $system \      # set "$system"
	#   --slurpfile manifest "$1" \ # slurp json into "$manifest"
	local jqargs=("-n" "-e" "-r" "-f" "$_lib/manifest.jq")

	# N.B jq invocation aborts if it cannot slurp a file, so if the registry
	# doesn't already exist (with nonzero size) then replace with bootstrap.
	if [ -s "$manifest" ]; then
		jqargs+=("--slurpfile" "manifest" "$manifest")
	else
		jqargs+=("--argjson" "manifest" '[{"elements": [], "version": 1}]')
	fi

	# Append arg which defines $system.
	jqargs+=("--arg" "system" "$FLOX_SYSTEM")

	# Append remaining args using jq "--args" flag and "--" to
	# prevent jq from interpreting provided args as options.
	jqargs+=("--args" "--" "$@")

	# Finally invoke jq.
	minverbosity=2 $invoke_jq "${jqargs[@]}"
}

#
# manifestTOML(command,[args])
#
# Accessor method for declarative TOML manifest library functions.
# Expects to read a manifest.toml passed in STDIN.
#
function manifestTOML() {
	trace "$@"
	# jq args:
	#   -r \                        # raw output (i.e. don't add quotes)
	#   -f $_lib/manifest.jq \      # the manifest processing library
	#   --arg system $system \      # set "$system"
	#   --slurpfile manifest "$1" \ # slurp json into "$manifest"
	local jqargs=("-r" "-f" "$_lib/manifestTOML.jq")

	# Add "slurp" mode for pulling manifest from STDIN.
	jqargs+=("-s")

	# Append various args.
	jqargs+=("--arg" "system" "$FLOX_SYSTEM")
	jqargs+=("--argjson" "verbose" "$verbose")
	jqargs+=("--arg" "environmentOwner" "$environmentOwner")
	jqargs+=("--arg" "environmentName" "$environmentName")
	jqargs+=("--arg" "FLOX_PATH_PREPEND" "$FLOX_PATH_PREPEND")

	# Append remaining args using jq "--args" flag and "--" to
	# prevent jq from interpreting provided args as options.
	jqargs+=("--args" "--" "$@")

	# Finally invoke jq.
	minverbosity=2 $invoke_dasel -f - -r toml -w json | $invoke_jq "${jqargs[@]}"
}

#
# renderManifestTOML(path/to/manifest.toml)
#
# Invokes commands to create a profile package from the supplied
# manifest.toml file. To be replaced by renderFloxEnv() someday soon.
#
function renderManifestTOML() {
	trace "$@"
	local manifest_toml="$1"; shift

	# Derive a list of Nix installables.
	local -a installables=($($_cat $manifest_toml | manifestTOML installables))

	# Convert this list of installables to a list of floxpkgArgs.
	local -a floxpkgArgs
	for i in "${installables[@]}"; do
		floxpkgArgs+=("$(floxpkgArg "$i")")
	done

	if [ ${#floxpkgArgs[@]} -gt 0 ]; then
		# Now we use this list of floxpkgArgs to create a temporary profile.
		local tmpdir
		tmpdir=$(mkTempDir)
		$invoke_nix profile install --impure --profile $tmpdir/profile "${floxpkgArgs[@]}"

		# If we've gotten this far we have a profile. Follow the links to
		# identify the package, then (carefully) discard the tmpdir.
		environmentPackage=$(cd $tmpdir && readlink $(readlink profile))
		$_rm -f $tmpdir/profile $tmpdir/profile-1-link
		$_rmdir $tmpdir
		if [ -n "$environmentPackage" ]; then
			echo $environmentPackage
		else
			error "failed to render new environment" </dev/null
		fi
	else
		error "rendered empty environment" < /dev/null
	fi
}

# boolPrompt($prompt, $default)
#
# Displays prompt, collects boolean "y/n" response,
# returns 0 for yes and 1 for no.
function boolPrompt() {
	trace "$@"
	local prompt="$1"; shift
	local default="$1"; shift
	local defaultLower
	defaultLower=$(echo $default | $_tr A-Z a-z)
	local defaultrc
	case "$defaultLower" in
	n|no) defaultrc=1 ;;
	y|yes) defaultrc=0 ;;
	*)
		error "boolPrompt() called with invalid default" < /dev/null
		;;
	esac
	[ $interactive -eq 1 ] || return $defaultrc
	local defaultCaps
	defaultCaps=$(echo $default | tr a-z A-Z)
	local defaultPrompt
	defaultPrompt=$(echo "y/n" | tr "$defaultLower" "$defaultCaps")
	local value
	read -e -p "$prompt ($defaultPrompt) " value
	local valueLower
	valueLower=$(echo $value | tr A-Z a-z)
	case "$valueLower" in
	n|no) return 1 ;;
	y|yes) return 0 ;;
	"") return $defaultrc ;;
	*)
		echo "invalid response \"$value\" .. try again" 1>&2
		boolPrompt "$prompt" "$default"
		;;
	esac
}

# promptInput($prompt, $value)
#
# Fancy gum invocation with --width set properly.
function promptInput() {
	trace "$@"
	local placeholder="$1"; shift
	local prompt="$1"; shift
	local value="$1"; shift
	# If not interactive then go with the default.(?)
	[ $interactive -eq 1 ] || {
		echo "$value"
		return 0
	}
	# Just assume a reasonable(?) screen width if COLUMNS not set.
	local -i columns=${COLUMNS:-80}
	local -i width
	width=$(( $columns - ${#prompt} ))
	if [ $width -gt 0 ]; then
		$_gum input --placeholder "$placeholder" --prompt "$prompt " --value "$value" --width $width
	else
		# If the math doesn't work then let gum choose what to do.
		$_gum input --placeholder "$placeholder" --prompt "$prompt " --value "$value"
	fi
}

# gitConfigSet($varname, $default)
function gitConfigSet() {
	trace "$@"
	local varname="$1"; shift
	local prompt="$1"; shift
	local default="$1"; shift
	local value="$default"
	while true
	do
		read -e -p "$prompt" -i "$value" value
		if boolPrompt "OK to invoke: 'git config --global $varname \"$value\"'" "yes"; then
			$_git config --global "$varname" "$value"
			break
		else
			info "OK, will try that again"
		fi
	done
}

#
# registry(registry,command,[args])
#
# Accessor method for jq-based registry library functions.
#
# Usage:
#   registry path/to/registry.json 1 (set|setString) a b c
#   registry path/to/registry.json 1 setNumber a b 3
#   registry path/to/registry.json 1 delete a b c
#   registry path/to/registry.json 1 (addArray|addArrayString) d e f
#   registry path/to/registry.json 1 addArrayNumber d e 6
#   registry path/to/registry.json 1 (delArray|delArrayString) d e f
#   registry path/to/registry.json 1 delArrayNumber d e 6
#   registry path/to/registry.json 1 get a b
#   registry path/to/registry.json 1 dump
#
# Global variable for prompting to confirm existing values.
declare -i getPromptSetConfirm=0
function registry() {
	trace "$@"
	local registry="$1"; shift
	local version="$1"; shift

	# The "getPromptSet" subcommand is a special-case function which
	# first attempts to get a value and if not found will then
	# prompt the user with a default value to set.
	if [ "$1" = "getPromptSet" ]; then
		shift
		local prompt="$1"; shift
		local defaultVal="$1"; shift
		local value
		value=$(registry "$registry" "$version" "get" "$@" || true)
		if [ -z "$value" ]; then
			read -e -p "$prompt" -i "$defaultVal" value
			registry "$registry" "$version" "set" "$@" "$value"
		elif [ $getPromptSetConfirm -gt 0 ]; then
			read -e -p "$prompt" -i "$value" value
			registry "$registry" "$version" "set" "$@" "$value"
		fi
		echo "$value"
		return
	fi

	# jq args:
	#   -S \                        # sort keys for stable output
	#   -n \                        # null input
	#   -e \                        # exit nonzero on errors
	#   -r \                        # raw output (i.e. don't add quotes)
	#   -f $_lib/registry.jq \      # the registry processing library
	#   --slurpfile registry "$1" \ # slurp json into "$registry"
	#	--arg version "$2" \        # required schema version
	local jqargs=("-S" "-n" "-e" "-r" "-f" "$_lib/registry.jq" "--arg" "version" "$version")

	# N.B jq invocation aborts if it cannot slurp a file, so if the registry
	# doesn't already exist (with nonzero size) then replace with bootstrap.
	if [ -s "$registry" ]; then
		jqargs+=("--slurpfile" "registry" "$registry")
	else
		jqargs+=("--argjson" "registry" "[{\"version\": $version}]")
	fi

	# Append remaining args using jq "--args" flag and "--" to
	# prevent jq from interpreting provided args as options.
	jqargs+=("--args" "--" "$@")

	case "$1" in
		# Methods which update the registry.
		set | setNumber | setString | \
		addArray | addArrayNumber | addArrayString | \
		delete | delArray | delArrayNumber | delArrayString)
			local _tmpfile
			_tmpfile=$(mkTempFile)
			minverbosity=2 $invoke_jq "${jqargs[@]}" > $_tmpfile
			if [ -s "$_tmpfile" ]; then
				$_cmp -s $_tmpfile $registry || $_mv $_tmpfile $registry
				$_rm -f $_tmpfile
				local dn
				dn=$($_dirname $registry)
			else
				error "something went wrong" < /dev/null
			fi
		;;

		# All others return data from the registry.
		*)
			minverbosity=2 $invoke_jq "${jqargs[@]}"
		;;
	esac
}

#
# initFloxUserMetaJSON($workDir)
#
# Accepts content on STDIN and creates initial floxUserMeta.json.
#
function initFloxUserMetaJSON() {
	trace "$@"
	local message="$1"; shift

	# First verify that the clone exists with a defined origin.
	# Note that this function will bootstrap the clone into existence.
	local origin
	origin=$(getSetOrigin "$defaultEnv")

	# We normally use an ephemeral clone in floxUserMetaRegistry() to
	# modify floxUserMeta.json but for bootstrapping it's OK to use the
	# existing clone in situ as it's designed to have floxmain checked
	# out at all times anyway.
	local workDir="$userFloxMetaCloneDir"

	# Start by checking out the floxmain branch, which is guaranteed to
	# exist because it's found in github:flox/floxmeta-template.
	$invoke_git -C "$workDir" checkout --quiet "$defaultBranch"

	# Check for uncommitted file in the way.
	if [ -f "$workDir"/floxUserMeta.json ]; then
		$_mv --verbose "$workDir"/floxUserMeta.json{,.$now}
	fi

	# Capture STDIN to the new file.
	$_cat > "$workDir"/floxUserMeta.json

	# Record unique client UUID.
	registry "$workDir"/floxUserMeta.json 1 get floxClientUUID >/dev/null 2>&1 || \
		registry "$workDir"/floxUserMeta.json 1 set floxClientUUID $($_uuid)

	# Add and commit.
	$invoke_git -C "$workDir" add floxUserMeta.json
	$invoke_git -C "$workDir" commit -m "$message" --quiet
}

#
# function floxUserMetaRegistry()
#
# Get or modify data in $userFloxMetaCloneDir:floxmain:floxUserMeta.json
# in a transaction.
#
function floxUserMetaRegistry() {
	trace "$@"
	local verb="$1"; shift

	# Recall that the git version of floxUserMeta.json is copied to the
	# $floxUserMeta file in bootstrap(). If that file is empty then we
	# know to initialize the file in git and follow that by refreshing
	# the temporary $floxUserMeta file in the local filesystem.
	if [ ! -s $floxUserMeta ]; then
		local floxUserMetaTemplate='{"channels":{}, "version":1}'
		if [ -f $OLDfloxUserMeta ]; then
			# XXX TEMPORARY: migrate data from ~/.config/floxUserMeta.json.
			# XXX Delete after 20230331.
			$_jq -r -S --argjson floxUserMetaTemplate "$floxUserMetaTemplate" '
				del(.channels."flox") |
				del(.channels."nixpkgs") |
				del(.channels."nixpkgs-flox") as $old |
				( $floxUserMetaTemplate * $old )
			' $OLDfloxUserMeta |
				initFloxUserMetaJSON "init: floxUserMeta.json (migrated from <=0.0.9)"
		else
			$_jq -n -r -S "$floxUserMetaTemplate" |
				initFloxUserMetaJSON "init: floxUserMeta.json"
		fi
		$_git -C "$userFloxMetaCloneDir" show "$defaultBranch:floxUserMeta.json" >$floxUserMeta
	fi

	# XXX TEMPORARY: write back contents to $OLDfloxUserMeta while we work
	# to update the rust CLI to read this information from git.
	if [ ! -f $OLDfloxUserMeta ]; then
		$_cp -f $floxUserMeta $OLDfloxUserMeta
	fi

	case "$verb" in
	get|dump)
		# Perform the registry query.
		registry $floxUserMeta 1 "$verb" $@
		;;
	set|setNumber|delete)
		# Create ephemeral clone.
		local workDir
		workDir=$(mkTempDir)
		$_git clone --quiet --shared "$userFloxMetaCloneDir" $workDir
		# Check out the floxmain branch in the ephemeral clone.
		$_git -C "$workDir" checkout --quiet $defaultBranch
		# Modify the registry file
		registry "$workDir/floxUserMeta.json" 1 "$verb" $@
		$_git -C $workDir add "floxUserMeta.json"
		$_git -C $workDir commit -m "$invocation_string" --quiet
		$_git -C $workDir push --quiet --set-upstream origin $defaultBranch
		# Refresh temporary $floxUserMeta (used for this invocation only).
		$_git -C "$userFloxMetaCloneDir" show "$defaultBranch:floxUserMeta.json" >$floxUserMeta
		# XXX TEMPORARY: write back contents to $OLDfloxUserMeta while we work
		# to update the rust CLI to read this information from git.
		$_cp -f $floxUserMeta $OLDfloxUserMeta
		;;
	*)
		error "floxUserMetaRegistry(): unsupported operation '$verb'" </dev/null
	esac
}

#
# environmentRegistry($environment,command,[args])
# XXX refactor; had to duplicate above to add $environmentName.  :-\
#
function environmentRegistry() {
	trace "$@"
	local workDir="$1"; shift
	local environment="$1"; shift
	local registry="$workDir/metadata.json"
	# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
	eval $(decodeEnvironment "$environment")
	local version=1

	# jq args:
	#   -n \                        # null input
	#   -e \                        # exit nonzero on errors
	#   -r \                        # raw output (i.e. don't add quotes)
	#   -f $_lib/registry.jq \      # the registry processing library
	#   --slurpfile registry "$1" \ # slurp json into "$registry"
	#	--arg version "$2" \        # required schema version
	local jqargs=(
		"-n" "-e" "-r" "-f" "$_lib/environmentRegistry.jq"
		"--argjson" "now" "$now"
		"--arg" "version" "$version"
		"--arg" "environmentParentDir" "$environmentParentDir"
		"--arg" "environmentName" "$environmentName"
		"--arg" "environmentSystem" "$environmentSystem"
		"--arg" "environmentMetaDir" "$workDir"
	)

	# N.B jq invocation aborts if it cannot slurp a file, so if the registry
	# doesn't already exist (with nonzero size) then replace with bootstrap.
	if [ -s "$registry" ]; then
		jqargs+=("--slurpfile" "registry" "$registry")
	else
		jqargs+=("--argjson" "registry" "[{\"version\": $version, \"generations\": {}}]")
	fi

	# Append remaining args using jq "--args" flag and "--" to
	# prevent jq from interpreting provided args as options.
	jqargs+=("--args" "--" "$@")

	case "$1" in
		# Methods which update the registry.
		set | setNumber | setString | \
		addArray | addArrayNumber | addArrayString | \
		delete | delArray | delArrayNumber | delArrayString)
			local _tmpfile
			_tmpfile=$(mkTempFile)
			$invoke_jq "${jqargs[@]}" > $_tmpfile
			if [ -s "$_tmpfile" ]; then
				$_cmp -s $_tmpfile $registry || $_mv $_tmpfile $registry
				$_rm -f $_tmpfile
				local dn
				dn=$($_dirname $registry)
				[ ! -e "$dn/.git" ] || \
					$_git -C $dn add $($_basename $registry)
			else
				error "something went wrong" < /dev/null
			fi
		;;

		# All others return data from the registry.
		*)
			$invoke_jq "${jqargs[@]}"
		;;
	esac
}

#
# multChoice($prompt $thing)
#
# usage: multChoice "Your favorite swear variable" "variable" \
#   "foo: description of foo" "bar: description of bar"
#
function multChoice {
	trace "$@"

	local prompt="$1"; shift
	local thing="$1"; shift
	# ... choices follow in "$@"

	local -a _choices

	echo 1>&2
	echo "$prompt" 1>&2
	_choices=($(
		local -i count=0
		while [ $# -gt 0 ]
		do
			let ++count
			# Prompt user to STDERR
			echo "$count) $1" 1>&2
			# Echo choice to STDOUT
			echo "${1//:*/}"
			shift
		done
	))

	local choice
	while true
	do
		read -e -p "Choose $thing by number: " choice
		choice=$((choice + 0)) # make int
		if [ $choice -gt 0 -a $choice -le ${#_choices[@]} ]; then
			index=$(($choice - 1))
			echo "${_choices[$index]}"
			return
		fi
		info "Incorrect choice try again"
	done
	# Not reached
}

#
# environmentArg($arg)
#
# Returns path to flox-managed link to be included in PATH.
#
function environmentArg() {
	trace "$@"
	# flox environments must resolve to fully-qualified paths within
	# $FLOX_ENVIRONMENTS. Resolve paths in a variety of ways:
	if [[ "$1" =~ '#' ]]; then
		# Project environment. Pass through flakeref unaltered.
		echo "$1"
	elif [[ ${1:0:1} = "/" ]]; then
		if [[ "$1" =~ ^$FLOX_ENVIRONMENTS ]]; then
			# Path already a floxpm profile - use it.
			echo "$1"
		elif [[ -L "$1" ]]; then
			# Path is a link - try again with the link value.
			echo $(environmentArg "$(readlink "$1")")
		else
			error "\"$1\" is not a flox environment path" < /dev/null
		fi
	elif [[ "$1" =~ \	|\  ]]; then
		error "environment \"$1\" cannot contain whitespace" < /dev/null
	else
		local old_ifs="$IFS"
		local IFS=/
		local -a _parts=($1)
		IFS="$old_ifs"
		if [ ${#_parts[@]} -eq 1 ]; then
			# Return default path for the environment directory.
			echo "$FLOX_ENVIRONMENTS/$defaultEnvironmentOwner/$FLOX_SYSTEM.${_parts[0]}"
		elif [ ${#_parts[@]} -eq 2 ]; then
			# Return default path for the environment directory.
			echo "$FLOX_ENVIRONMENTS/${_parts[0]}/$FLOX_SYSTEM.${_parts[1]}"
		else
			usage | error "invalid environment \"$1\""
		fi
	fi
}

function checkValidSystem() {
	case "$1" in
	aarch64-linux|aarch64-darwin|i686-linux|x86_64-linux|x86_64-darwin)
		return 0
		;;
	esac
	return 1
}

#
# decodeEnvironment($environment)
#
# Parses environment path and returns code to define:
#   $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
#
# Called with: eval $(decodeEnvironment $environment)
#
function decodeEnvironment() {
	trace "$@"
	local environment="$1"; shift

	# Other variables to be populated based on environment type.
	local branchName=""
	local floxNixDir=""
	local environmentAlias=""
	local environmentSystem=""
	local environmentBaseDir=""
	local environmentBinDir=""
	local environmentMetaDir=""
	local environmentParentDir=""
	local environmentName=""
	local environmentOwner=""

	if [[ "$environment" =~ ^$FLOX_ENVIRONMENTS ]]; then # named flox environment

		# First parse the straightforward "/"-delimited values.
		local _old_ifs="$IFS"
		local IFS=/
		local -a _parts=($environment)
		local _numParts=${#_parts[@]}
		environmentParentDir="${_parts[*]:0:$(($_numParts-1))}" # == dirname $environment
		environmentName="${_parts[$(($_numParts-1))]}" # == basename $environment
		environmentOwner="${_parts[$(($_numParts-2))]}" # == basename $environmentParentDir
		IFS="$_old_ifs"

		environmentBaseDir="$environment"
		environmentBinDir="$environment/bin"
		environmentMetaDir="$FLOX_META/$environmentOwner"

		# The first part of environmentName is the system.
		environmentSystem="${environmentName/\.*/}"
		if ! checkValidSystem "$environmentSystem"; then
			# Assume we're only on a single system while we transition to the
			# new directory layout (with system included in the PATH).
			# echo "could not decode '$environment', invalid system type: '$environmentSystem'" </dev/null
			environmentSystem=$NIX_CONFIG_system
		fi

		# What remains is the alias (can contain dots). If this environment
		# is not owned by $defaultEnvironmentOwner then also prepend the
		# owner to the alias.
		environmentAlias="${environmentName/$environmentSystem\./}"
		if [ "$environmentOwner" != "$defaultEnvironmentOwner" ]; then
			environmentAlias="$environmentOwner/$environmentAlias"
		fi

		# $branchName was previously a calculated field but is now the same
		# as $environmentName. Nevertheless we'll keep it as it's useful
		# to know in what context we're using the string.
		local branchName="$environmentName"

	elif [[ "$environment" =~ '#' ]]; then # project flox environment

		local installableFlakeRef=${environment//#*/} # aka $topLevel
		local installableAttrPath=${environment//*#/}
		installableAttrPath="${installableAttrPath//.floxEnvs.$FLOX_SYSTEM./}"
		local topLevel
		topLevel=$(flakeTopLevel "$installableFlakeRef" "${invocationArgs[@]}")
		local metaDir
		metaDir=$(flakeMetaDir "$topLevel")

		# Similarly parse $topLevel "/"-delimited values.
		local _old_ifs="$IFS"
		local IFS=/
		local -a _parts=($topLevel)
		local _numParts=${#_parts[@]}
		environmentParentDir="${_parts[*]:0:$(($_numParts-1))}" # == dirname $topLevel
		environmentName="${_parts[$(($_numParts-1))]}#$installableAttrPath"
		IFS="$_old_ifs"

		environmentBaseDir="$metaDir/envs/$FLOX_SYSTEM.$installableAttrPath"

		local floxEnvFlakeURL="${installableFlakeRef}#.floxEnvs.$FLOX_SYSTEM.$installableAttrPath"
		local floxNixStorePath
		floxNixStorePath="$($invoke_nix eval "$floxEnvFlakeURL".meta.position --impure --raw 2>/dev/null || true)"
		[ -n "$floxNixStorePath" ] || \
			error "could not determine directory for '$installableFlakeRef'" < /dev/null
		local floxNixDir
		floxNixDir="$topLevel/${floxNixStorePath#/*/*/*/}"
		floxNixDir="$($_dirname "$floxNixDir")"

		environmentAlias=".#$installableAttrPath"
		environmentSystem="$FLOX_SYSTEM"
		environmentBinDir="$environmentBaseDir/bin"

	else

		error "could not identify type of environment '$environment'" </dev/null

	fi

	for i in branchName floxNixDir environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}; do
		echo "local $i='${!i}'"
	done
}

# Package args can take one of the following formats:
# 1) flake references containing "#" character: return as-is.
# 2) positional integer references containing only numbers [0-9]+.
# 3) paths which resolve to /nix/store/*: return first 3 path components.
# 4) floxpkgs "[[stability.]channel.]attrPath" tuple: convert to flox catalog
#    flake reference, e.g.
#      stable.nixpkgs-flox.yq ->
#        flake:nixpkgs-flox#catalog.aarch64-darwin.stable.yq
function versionedFloxpkgArg() {
	trace "$@"
	if [[ "$1" == *#* ]]; then
		echo "$1"
	elif [[ "$1" =~ ^[0-9]+$ ]]; then
		echo "$1"
	elif [ -e "$1" ]; then
		_rp=$($_realpath "$1")
		if [[ "$_rp" == /nix/store/* ]]; then
			echo "$_rp" | $_cut -d/ -f1-4
		fi
	else
		# Derive fully-qualified floxTuple.
		local IFS='.'
		local -a input=($1)
		local floxTuple=
		case "${input[0]}" in
		stable | staging | unstable)
			if [ ${validChannels["${input[1]}"]+_} ]; then
				# stability.channel.attrPath
				# They did all the work for us.
				floxTuple="$1"
			else
				# stability.attrPath .. perhaps we shouldn't support this?
				# Inject "nixpkgs-flox" as the default channel.
				floxTuple="${input[0]}.nixpkgs-flox.${input[@]:1}"
			fi
			;;
		*)
			if [ ${validChannels["${input[0]}"]+_} ]; then
				# channel.attrPath
				floxTuple="stable.$1"
			else
				# attrPath
				floxTuple="stable.nixpkgs-flox.$1"
			fi
			;;
		esac

		# Convert fully-qualified floxTuple:
		#   "<stability>.<channel>.<attrPath>"
		# to flakeref:
		#   "flake:<channel>#evalCatalog.${FLOX_SYSTEM}.<stability>.<attrPath>".
		local flakeref=
		local -a _floxTuple=($floxTuple)
		flakeref="flake:${_floxTuple[1]}#evalCatalog.${FLOX_SYSTEM}.${_floxTuple[0]}.${_floxTuple[@]:2}"

		# Return flakeref.
		echo "$flakeref"
	fi
}

function floxpkgArg() {
	trace "$@"
	local flakeref
	flakeref=$(versionedFloxpkgArg "$@")

	# Convert "attrPath@x.y.z" to "attrPath.x_y_z" because that
	# is how it appears in the flox catalog.
	if [[ "$flakeref" =~ ^(.*)@(.*)$ ]]; then
		flakeref="${BASH_REMATCH[1]}.${BASH_REMATCH[2]//[\.]/_}"
	fi

	# Return flakeref.
	echo "$flakeref"
}

#
# nixEditor($environment, $floxNix, "(install|delete)", $versionedFloxpkgArg)
#
# Takes a path to a flox.nix file, the command "install" or "delete",
# and a versioned floxpkg arg to look for in the flox.nix.
#
function nixEditor() {
	trace "$@"
	local environment="$1"; shift
	local floxNix="$1"; shift
	local action="$1"; shift
	local versionedFloxpkgArg="$1"; shift
	local -a nixEditorArgs
	# FIXME: the use of read() below exits nonzero because of EOF.
	IFS=$'\n' read -r -d '' -a nixEditorArgs < \
		<(manifest $environment/manifest.json floxpkgToNixEditorArgs "$versionedFloxpkgArg") || :
	# That's it; invoke the editor to add the package.
	case "$action" in
	install)
		$invoke_nix_editor -i $workDir/$nextGen/pkgs/default/flox.nix "${nixEditorArgs[@]}"
		;;
	delete)
		# TODO: if a user tries to remove a package with the version specified,
		#       but the package was installed without the version specified,
		#       this won't work
		# TODO: this only removes the first instance of a package it finds.
		#       We may need to enforce there's only one instance of a package
		#       on the module side
		# ignore args after the first one, since the rest are used for installation
		$invoke_nix_editor -id $workDir/$nextGen/pkgs/default/flox.nix "${nixEditorArgs[0]}"
		;;
	esac
}

#
# Rudimentary pattern-matching URL parser.
# Surprised there's no better UNIX command for this.
#
# Usage:
#	local urlTransport urlHostname urlUsername
#	eval $(parseURL "$url")
#
function parseURL() {
	trace "$@"
	local url="$1"; shift
	local urlTransport urlHostname urlUsername
	case "$url" in
	git+ssh@*:) # e.g. "git+ssh@github.com:"
		urlTransport="${url//@*/}"
		urlHostname="${url//*@/}"
		urlHostname="${urlHostname//:*/}"
		urlUsername="git"
		;;
	https://*|http://*) # e.g. "https://github.com/"
		urlTransport="${url//:*/}"
		urlHostname="$(echo $url | $_cut -d/ -f3)"
		urlUsername=""
		;;
	*)
		error "parseURL(): cannot parse \"$url\"" < /dev/null
		;;
	esac
	echo urlTransport="\"$urlTransport\""
	echo urlHostname="\"$urlHostname\""
	echo urlUsername="\"$urlUsername\""
}

#
# Convert git_base_url to URL for use in flake registry.
#
# Flake URLs specify branches in different ways,
# e.g. these are all equivalent:
#
#   git+ssh://git@github.com/flox/floxpkgs?ref=master
#   https://github.com/flox/floxpkgs/archive/master.tar.gz
#   github:flox/floxpkgs/master
#
# Usage:
#	git_base_urlToFlakeURL ${git_base_url} ${organization}/floxpkgs master
#
function git_base_urlToFlakeURL() {
	trace "$@"
	local baseurl="$1"; shift
	local path="$1"; shift
	local ref="$1"; shift
	# parseURL() emits commands to set urlTransport, urlHostname and urlUsername.
	local urlTransport urlHostname urlUsername
	eval $(parseURL "$baseurl") || \
		error "cannot convert to flake URL: \"$baseurl\"" < /dev/null
	case $urlTransport in
	https|http)
		case $urlHostname in
		github.com)
			echo "github:$path/$ref"
			;;
		*)
			echo "$urlTransport://${urlUsername:+$urlUsername@}$urlHostname/$path/$ref"
			;;
		esac
		;;
	git+ssh)
		echo "$urlTransport://${urlUsername:+$urlUsername@}$urlHostname/$path?ref=$ref"
		;;
	esac
}

# validateTOML(path)
function validateTOML() {
	trace "$@"
	local path="$1"; shift
	# XXX do more here to highlight what the problem is.
	tmpstderr=$(mkTempFile)
	if $_cat $path | $_dasel -r toml -w toml >/dev/null 2>$tmpstderr; then
		: confirmed valid TOML
		$_rm -f $tmpstderr
		return 0
	else
		warn "'$path' contains invalid TOML syntax - see below:"
		$_cat $tmpstderr 1>&2
		$_rm -f $tmpstderr
		echo "" 1>&2
		return 1
	fi
}

# validateFlakeURL()
#
# Perform basic sanity check of FlakeURL to make sure it exists.
function validateFlakeURL() {
	trace "$@"
	local flakeURL="$1"; shift
	if $invoke_nix flake metadata "$flakeURL" --no-write-lock-file --json >/dev/null; then
		return 0
	else
		return 1
	fi
}

#
# getChannelsJSON()
#
# Merge user-subscribed and flox-provided channels in a single JSON stream.
#
function getChannelsJSON() {
	trace "$@"
	# Combine flox-provided and user channels in a single stream.
	# Be careful to handle the case where user registry is corrupt or
	# missing the 'channels' hash.
	( floxUserMetaRegistry get channels || echo '{}' ) | $_jq -S -r '
		( . | with_entries(.value={type:"user",url:.value}) ) as $userChannels |
		(
		  {
		    "flox": "github:flox/floxpkgs/master",
		    "nixpkgs-flox": "github:flox/nixpkgs-flox/master"
		  } | with_entries(.value={type:"flox",url:.value})
		) as $floxChannels |
		( $floxChannels * $userChannels )
	'
}

# Populate user-specific flake registry.
function updateFloxFlakeRegistry() {
	trace "$@"
	# Render Nix flake registry using flox and user-provided entries.
	# Note: avoids problems to let nix create the temporary file.
	tmpFloxFlakeRegistry=$($_mktemp --dry-run --tmpdir=$FLOX_CONFIG_HOME)
	. <(getChannelsJSON | $_jq -r '
	  to_entries | sort_by(.key) | map(
	    "minverbosity=2 $invoke_nix registry add --registry $tmpFloxFlakeRegistry \"\(.key)\" \"\(.value.url)\" && validChannels[\(.key)]=\"\(.value.type)\""
	  )[]
	')

	# Add courtesy Nix flake entries for accessing nixpkgs of different stabilities.
	# We provide these as a backup to the use of "nixpkgs/{stable,staging,unstable}"
	# in the event that the user overrides the "nixpkgs" entry in their user registry.
	# We add this at the flake level, but we don't include them in getChannelsJSON
	# above because these aren't "channels" containing flox catalogs.
	minverbosity=2 $invoke_nix registry add --registry $tmpFloxFlakeRegistry nixpkgs github:flox/nixpkgs/$FLOX_STABILITY
	minverbosity=2 $invoke_nix registry add --registry $tmpFloxFlakeRegistry nixpkgs-stable github:flox/nixpkgs/stable
	minverbosity=2 $invoke_nix registry add --registry $tmpFloxFlakeRegistry nixpkgs-staging github:flox/nixpkgs/staging
	minverbosity=2 $invoke_nix registry add --registry $tmpFloxFlakeRegistry nixpkgs-unstable github:flox/nixpkgs/unstable

	# order of keys is not relevant for json data
	if [ -f $floxFlakeRegistry ] && $_cmp --quiet <($_jq -S < $tmpFloxFlakeRegistry) <($_jq -S < $floxFlakeRegistry); then
		$_rm $tmpFloxFlakeRegistry
	else
		$_mv -f $tmpFloxFlakeRegistry $floxFlakeRegistry
	fi
}

#
# searchChannels($regexp)
#
function searchChannels() {
	trace "$@"
	local regexp="$1"; shift
	# XXX Passing optional arguments with bash is .. problematic.
	# XXX Walk through the remaining arguments looking for options
	# XXX and valid channel references.
	local refreshArg
	local -a channels=()
	while test $# -gt 0; do
		case "$1" in
		--refresh)
			refreshArg="--refresh"
			;;
		*)
			if [ ${validChannels["$1"]+_} ]; then
				channels+=("$1")
			else
				error "invalid channel: $1" < /dev/null
			fi
			;;
		esac
		shift
	done

	# If no channels were requested then search all of them.
	if [ ${#channels[@]} -eq 0 ]; then
		channels=(${!validChannels[@]})
	fi

	# Refresh channel data before searching, but don't refresh the
	# nixpkgs-flox channel by default because it is expensive to update and
	# doesn't change often, unlike other channels that are quicker to update
	# and for which people expect to see updates reflected instantly.
	for channel in ${channels[@]}; do
		[ $channel != "nixpkgs-flox" ] || continue
		$invoke_nix flake metadata "flake:${channel}" --refresh ${_nixArgs[@]}  > /dev/null
	done

	# Construct temporary script for performing search in parallel.
	# TODO: use log-format internal-json for conveying status
	local _script
	_script=$(mkTempFile)
	local _tmpdir
	_tmpdir=$(mkTempDir)
	local -a _channelDirs=($(for i in ${channels[@]}; do echo $_tmpdir/$i; done))
	local -a _resultDirs=($(for i in ${channels[@]}; do echo $_tmpdir/$i/{stable,staging,unstable}; done))
	local -a _stdoutFiles=($(for i in ${channels[@]}; do echo $_tmpdir/$i/{stable,staging,unstable}/stdout; done))
	local -a _stderrFiles=($(for i in ${channels[@]}; do echo $_tmpdir/$i/{stable,staging,unstable}/stderr; done))
	local _nixInvocationVariables=()
	for i in ${exported_variables[$_nix]}; do
		_nixInvocationVariables+=("$i=${!i}")
	done
	for channel in ${channels[*]}; do
		for stability in stable staging unstable; do
			$_mkdir -p $_tmpdir/$channel/$stability
			local -a cmd=(
				$_nix search --log-format bar --json --no-write-lock-file $refreshArg
				"'flake:${channel}#.catalog.${FLOX_SYSTEM}.$stability'" "'$packageregexp'"
			)
			echo "${cmd[@]} >$_tmpdir/$channel/$stability/stdout 2>$_tmpdir/$channel/$stability/stderr &" >> $_script
			[ $verbose -lt $minverbosity ] || warn "+ ${_nixInvocationVariables[@]} ${cmd[@]}"
		done
	done
	echo "wait" >> $_script
	if [ $interactive -eq 1 ]; then
		# gum BUG: writes the spinner to stdout (dumb) - redirect that to stderr
		$_gum spin --title="Searching channels: ${channels[*]}" 1>&2 -- $_bash $_script
	else
		$_bash $_script
	fi

	# The results directory is composed of files of the form:
	#     <channel>/{stdout,stderr}
	# Use jq to compile a single json stream from results.
	$_grep --no-filename -v \
	  -e "^evaluating 'catalog\." \
	  -e "not writing modified lock file of flake" \
	  -e ".sqlite' is busy" \
	  -e " Added input " \
	  -e " follows " \
	  -e "\([0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]\)" \
	  ${_stderrFiles[@]} 1>&2 || true
	$invoke_jq -r -f "$_lib/merge-search-results.jq" ${_stdoutFiles[@]} | \
		$_jq -r -s add
	if [ $debug -eq 0 ]; then
		$_rm -f ${_stdoutFiles[@]}
		$_rm -f ${_stderrFiles[@]}
		$_rmdir ${_resultDirs[@]} ${_channelDirs[@]} $_tmpdir
	fi
}

#
# Prompts the user for attrPath to be built/published/etc.
#
function lookupAttrPaths() {
	trace "$@"
	local flakeRef=$1; shift
	local attrTypes=("$@"); shift
	for type in "${attrTypes[@]}"; do
		(minverbosity=2 $invoke_nix eval "$flakeRef#.$type.$FLOX_SYSTEM" --impure --json --apply builtins.attrNames 2>/dev/null || true)
	# Don't differentiate between identical attrpaths with different prefixes.
	# Technically that's incorrect, but it's currently desirable for develop.
	done | $_jq -s -r '. | add | unique[]'
}

function selectAttrPath() {
	trace "$@"
	local flakeRef="$1"; shift
	local subcommand="$1"; shift
	local attrTypes="$@"; shift
	local -a attrPaths=($(lookupAttrPaths $flakeRef $attrTypes))
	local attrPath
	if [ ${#attrPaths[@]} -eq 0 ]; then
		error "cannot find attribute path - have you run 'flox init'?" < /dev/null
	elif [ ${#attrPaths[@]} -eq 1 ]; then
		echo "${attrPaths[0]}"
	else
		warn "Select package for flox $subcommand"
		attrPath=$($_gum choose ${attrPaths[*]})

		local hintCommandArgs
		case "$subcommand" in
		activate|edit|install|list|remove|upgrade) hintCommandArgs="$subcommand -e .#$attrPath";;
		*) hintCommandArgs="$subcommand -A $attrPath";;
		esac

		warn ""
		warn "HINT: avoid selecting a package next time with:"
		echo '{{ Color "'$LIGHTPEACH256'" "'$DARKBLUE256'" "$ flox '$hintCommandArgs'" }}' \
		    | $_gum format -t template 1>&2
		echo "$attrPath"
	fi
}

function checkGitRepoExists() {
	trace "$@"
	local origin="$1"
	githubHelperGit ls-remote "$origin" >/dev/null 2>&1
}

function ensureGHRepoExists() {
	trace "$@"
	local origin="$1"
	local visibility="$2"
	local template="$3"
	# If using github, ensure that user is logged into gh CLI
	# and confirm that repository exists.
	if ! checkGitRepoExists "$origin"; then
		if [[ "${origin,,}" =~ github ]]; then
			( $_gh auth status >/dev/null 2>&1 ) ||
				$_gh auth login
			( $_gh repo view "$origin" >/dev/null 2>&1 ) || (
				set -x
				$_gh repo create \
					--"$visibility" "$origin" \
					--template "$template"
			)
		else
			false
		fi
	fi
}

# Appends new JSON metric to the $FLOX_METRICS file, then submits
# them all in a batch if the first entry in the file is older than
# an hour. Does not return.
function submitMetric() {
	trace "$@"
	local subcommand="$1"; shift
	[ $floxMetricsConsent -eq 1 ] || exit 0;
	[ -z "$FLOX_DISABLE_METRICS" ] || exit 0;

	# Blow away the file if it is faulty json.
	$_jq '.' "$FLOX_METRICS" >/dev/null 2>&1 || $_rm -f "$FLOX_METRICS"

	# Record the timestamp of the (non-empty) file before we change it.
	local file_timestamp=$now
	if [ -s "$FLOX_METRICS" ]; then
		file_timestamp=$($_stat -c %Y "$FLOX_METRICS")
	elif [ ! -f "$FLOX_METRICS" ]; then
		# Force sending of initial data when first creating file.
		local file_timestamp=0
	fi

	# Add new metric event for this invocation.
	# Note that jq's "--arg" flag performs safe quoting of strings.
	$_jq -n -r -c \
		--arg subcommand "$subcommand" \
		--arg floxClientUUID "$floxClientUUID" \
		--arg uuid "$($_uuid)" \
		--arg floxVersion '@@VERSION@@' \
		--arg OS "$($_uname -s | $_sed 's/^Darwin$/Mac OS/')" \
		--arg kernelVersion "$($_uname -r)" \
		--argjson now "$now" '
		{
			"event": "cli-invocation",
			"timestamp": $now,
			"uuid": "\($uuid)",
			"properties": {
				"distinct_id": "flox-cli:\($floxClientUUID)",
				"$device_id": "flox-cli:\($floxClientUUID)",

				"$os": "\($OS)",
				"kernel_version": "\($kernelVersion)",
				"flox_version": "\($floxVersion)",

				"$lib": "flox-cli",

				"subcommand": "\($subcommand)",
				"$current_url": "flox://\($subcommand)",
				"$pathname": "\($subcommand)",

				"$set_once": {
					"$initial_os": "\($OS)",
					"initial_kernel_version": "\($kernelVersion)",
					"initial_flox_version": "\($floxVersion)",
				},
				"$set": {
					"$os": "\($OS)",
					"kernel_version": "\($kernelVersion)",
					"flox_version": "\($floxVersion)",

					"flox-cli-uuid": "\($floxClientUUID)"
				}
			}
		}
	' >> "$FLOX_METRICS"

	# We don't want to send metrics too frequently, network noise sucks
	# so we don't ever send the batch out more than once an hour.
	# Set FLOX_SEND_METRICS=1 to force sending it anyway.
	if [ -z "$FLOX_SEND_METRICS" ]; then
		# If the file hasn't been modified for over an hour then always send.
		local -i time_since_file_modification
		time_since_file_modification=$(($now - $file_timestamp))
		if [ $time_since_file_modification -lt 3600 ]; then
			local -i first_timestamp_in_file
			first_timestamp_in_file=$($_jq -r 'if input_line_number == 1 then .timestamp else halt end' "$FLOX_METRICS")
			local -i time_since_first_event
			time_since_first_event=$(($now - $first_timestamp_in_file))
			[ $time_since_first_event -ge 3600 ] || exit 0
		fi
	fi

	# Create full JSON body to send to endpoint.
	minverbosity=2
	if $invoke_jq -n -r -c --slurpfile events "$FLOX_METRICS" '
		{
			"api_key": "phc_z4dOADAPvpU9VNzCjDD3pIJuSuGTyagKdFWfjak838Y",
			"batch": [
				$events[] | with_entries(
					if .key == "timestamp" then (
						.value |= ( . | strftime("%Y-%m-%dT%H:%M:%S%z") )
					) else . end
				)
			]
		}
	' | $_curl --silent -L --json @- "https://events.floxdev.com/capture" > /dev/null; then
		# Reset the batch file
		$_rm -f "$FLOX_METRICS"
		$_touch "$FLOX_METRICS"
	fi

	exit 0
}

#
# darwinRepairFiles()
#
# The flox installer includes logic to patch /etc/zshrc{,_Apple_Terminal}
# to fix bugs in Apple's default zshrc files, but it also seems that upgrades
# (and updates?) will indiscriminately blat user's customizations of these
# system-wide files. At some point we hope to get Apple to update their
# copies of the scripts, but in the meantime we check with each invocation
# to ensure that they haven't been reverted.
#
# https://github.com/flox/flox-bash-private/issues/434
#
function darwinPromptPatchFile() {
	trace "$@"
	brokenFile=$1; shift
	patchFile=$1; shift
	warn "flox modifications to '$brokenFile' for zsh session history support"
	warn "seem to have been reverted, possibly by way of a recent OS update."
	if $invoke_gum confirm --default="true" "Reapply flox patches to '$brokenFile'?"; then
		# Intentionally relying on Mac versions of sudo and patch.
		( set -x && \
			/usr/bin/sudo /usr/bin/patch -V none -p0 -d / --verbose < $patchFile ) || \
			warn "problems applying '$patchFile' - please reinstall flox"
	else
		warn "OK, note you may encounter problems with zsh session history."
	fi
}
function darwinRepairFiles() {
	trace "$@"
	[ $interactive -eq 1 ] || return 0
	# Only attempt to repair if /etc/zshrc* was patched at install time.
	if [ -f /etc/zshrc -a -f /etc/zshrc.backup-before-flox ] &&
		! $_grep -q 'HISTFILE=${HISTFILE:-${ZDOTDIR:-$HOME}/.zsh_history}' /etc/zshrc; then
		if $_cmp --quiet /etc/zshrc /etc/zshrc.backup-before-flox; then
			darwinPromptPatchFile /etc/zshrc $_share/flox/files/darwin-zshrc.patch
		else
			warn "broken 'HISTFILE' variable assignment in /etc/zshrc - please reinstall flox"
		fi
		warn "continuing ..."
	fi
	if [ -f /etc/zshrc_Apple_Terminal -a -f /etc/zshrc_Apple_Terminal.backup-before-flox ] &&
		! $_grep -q 'SHELL_SESSION_DIR="${SHELL_SESSION_DIR:-${ZDOTDIR:-$HOME}/.zsh_sessions}"' /etc/zshrc_Apple_Terminal; then
		if $_cmp --quiet /etc/zshrc_Apple_Terminal /etc/zshrc_Apple_Terminal.backup-before-flox; then
			darwinPromptPatchFile /etc/zshrc_Apple_Terminal $_share/flox/files/darwin-zshrc_Apple_Terminal.patch
		else
			warn "broken 'SHELL_SESSION_DIR' variable assignment in /etc/zshrc - please reinstall flox"
		fi
		warn "continuing ..."
	fi
}

#
# identifyParentShell()
#
# Do everything in our power to identify the parent shell. We basically
# only have two sources of information at our disposal:
#
# 1. the value of $SHELL, which may be a lie
# 2. the PID of the parent shell, as provided to us by the C/Rust wrapper
#    in the form of the FLOX_PARENT_PID environment variable
#
# Perform a sanity check to confirm that the value of $SHELL matches the
# current running shell. Algorithm is currently to
# compare the value of $0 to $SHELL, but we can go crazy later
# inspecting the running process, etc. if necessary.
#
function identifyParentShell() {
	trace "$@"
	local parentShell="$SHELL" # default
	local shellCmd="${SHELL/*\//}" # aka basename
	local parentShellCmd="$shellCmd" # default

	# Only attempt a guess if we know our parent PID.
	if [ -n "$FLOX_PARENT_PID" ]; then
		# First attempt to identify details of parent shell process.
		if [ -L "/proc/$FLOX_PARENT_PID/exe" -a \
			 -r "/proc/$FLOX_PARENT_PID/exe" ]; then
			# Linux - use information from /proc.
			parentShell="$($_readlink "/proc/$FLOX_PARENT_PID/exe")"
		elif local psOutput="$(ps -c -o command= -p $FLOX_PARENT_PID 2>/dev/null)"; then
			# Darwin/other - use `ps` to guess the shell.
			# Note that this value often comes back with a leading "-" character
			# that is not part of the executable's path.
			parentShell="${psOutput/#-/}"
		fi

		# Split out the command by itself (aka basename).
		parentShellCmd="${parentShell/*\//}"

		# Compare $SHELL and $parentShell to see if the command names match.
		if [ "$shellCmd" == "$parentShellCmd" ]; then
			# Respect $SHELL over $parentShell (which is usually its realpath).
			echo "$SHELL"
		else
			# Return parent shell.
			echo "$parentShell"
		fi
	else
		# We don't know our parent PID so don't even guess.
		echo "$SHELL"
	fi
}

#
# joinString( $separator, $string1, $string2, ... )
#
# Like any other join() function. Not calling it "join" because don't
# want to have confusion with coreutils `join`. Also remove dups.
#
function joinString() {
	trace "$@"
	local separator="$1"; shift
	local accum=
	local -A seen
	while test $# -gt 0; do
		while IFS=$separator read -r i; do
			case "$i" in
			"") : ;;
			*)
				if [ -z "${seen["$i"]}" ]; then
					if [ -n "$accum" ]; then
						accum="${accum}${separator}${i}"
					else
						accum="${i}"
					fi
					seen["$i"]=1
				fi
				;;
			esac
		done <<< $(echo "$1")
		shift
	done
	echo "$accum"
}

# vim:ts=4:noet:syntax=bash
