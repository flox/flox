# -*- mode: sh; sh-shell: bash; -*-
# Set prefix (again) to assist with debugging independently of flox.sh.
_prefix="@@PREFIX@@"
_prefix="${_prefix:-.}"
_lib="$_prefix/lib"
_libexec="$_prefix/libexec"
_etc="$_prefix/etc"

# Use extended glob functionality throughout.
shopt -s extglob

# Allow globs to return the empty list.
shopt -s nullglob

# TODO: One day we can turn these on...
# set -eu;
# set -o pipefail;

# Pull in utility functions early.
. "$_lib/utils.sh"

# Import library functions.
. "$_lib/metadata.sh"

# Import command functions.
. "$_lib/commands.sh"

#
# Parse flox configuration files in TOML format. Order of processing:
#
# 1. package defaults from $PREFIX/etc/flox.toml
# 2. installation defaults from /etc/flox.toml
# 3. user customizations from $FLOX_CONFIG_HOME/flox.toml
#
# Latter definitions override/redefine the former ones.
#
read_flox_conf()
{
	local _cline
	# Consider other/better TOML parsers. Calling dasel multiple times below
	# because it only accepts one query per invocation.  In benchmarks it claims
	# to be 3x faster than jq so this is better than converting to json in a
	# single invocation and then selecting multiple values using jq.
	for f in "$_prefix/etc/flox.toml" "/etc/flox.toml" "$FLOX_CONFIG_HOME/flox.toml"
	do
		if [[ -f "$f" ]]; then
		for i in "$@"
			do
				# Use `cat` to open files because it produces a clear and concise
				# message when file is not found or not readable. By comparison
				# the equivalent dasel output is to report "unknown parser".
				#
				# Use jq to look for the requested attribute because dasel always
				# returns nonzero when it is not found.
				#
				# Use the `jq` `tojson()` function to escape quotes contained in
				# values.
				#shellcheck disable=SC2016
				$_cat "$f" | $_dasel -r toml -w json \
					|$_jq -r --arg var "$i" 'if has($var) then "FLOX_CONF_\($var)=\(.[$var] | tojson)" else empty end'
			done
		fi
	done
}

nix_show_config()
{
	local -a _cline
	#shellcheck disable=SC2162
	$_nix show-config | while read -a _cline
	do
		if [[ -z "${_cline[*]}" ]]; then continue; fi
		case "${_cline[0]}" in
		# List below the parameters you want to use within the script.
		system)
			local _xline
			_xline=$(echo "${_cline[@]}" | $_tr -d ' \t')
			echo NIX_CONFIG_"$_xline"
			;;
		*)
			;;
		esac
	done
}

#
# Global variables
#

# NIX honors ${USER} over the euid, so make them match.
if _real_user=$($_id -un 2>/dev/null); then
	if [ "$_real_user" != "$USER" ]; then
		export USER="$_real_user"
		if _real_home=$($_getent passwd "$USER" 2>/dev/null | $_cut -d: -f6); then
			export HOME="$_real_home"
		else
			warn "cannot identify home directory for user '$USER'"
		fi
	fi
else
	# XXX Corporate LDAP environments rely on finding nss_ldap in
	# XXX ld.so.cache *or* by configuring nscd to perform the LDAP
	# XXX lookups instead. The Nix version of glibc has been modified
	# XXX to disable ld.so.cache, so if nscd isn't configured to do
	# XXX this then ldap access to the passwd map will not work.
	# XXX Bottom line - don't abort if we cannot find a passwd
	# XXX entry for the euid, but do warn because it's very
	# XXX likely to cause problems at some point.
	warn "cannot determine effective uid - continuing as user '$USER'"
fi
if [ -n "$HOME" ]; then
	[ -w "$HOME" ] || \
		error "\$HOME directory '$HOME' not writable ... aborting" < /dev/null
fi
export PWD=$($_pwd)

# Define and create flox metadata cache, data, and profiles directories.
export FLOX_STABILITY="${FLOX_STABILITY:-stable}"
export FLOX_CACHE_HOME="${FLOX_CACHE_HOME:-${XDG_CACHE_HOME:-$HOME/.cache}/flox}"
export FLOX_META="${FLOX_META:-$FLOX_CACHE_HOME/meta}"
export FLOX_DATA_HOME="${FLOX_DATA_HOME:-${XDG_DATA_HOME:-$HOME/.local/share}/flox}"
export FLOX_ENVIRONMENTS="${FLOX_ENVIRONMENTS:-$FLOX_DATA_HOME/environments}"
export XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
export FLOX_CONFIG_HOME="${FLOX_CONFIG_HOME:-$XDG_CONFIG_HOME/flox}"
$_mkdir -p "$FLOX_CACHE_HOME" "$FLOX_META" "$FLOX_DATA_HOME" "$FLOX_ENVIRONMENTS" "$FLOX_CONFIG_HOME"
for i in "$FLOX_CACHE_HOME" "$FLOX_META" "$FLOX_DATA_HOME" "$FLOX_ENVIRONMENTS" "$FLOX_CONFIG_HOME"; do
	# if $i is writable, do nothing, else try to create $i
	[ -w "$i" ] || $_mkdir -p "$i" || \
		error "directory '$i' not writable ... aborting" < /dev/null
done
export FLOX_VERSION="@@VERSION@@"

# Prepend FLOX_DATA_HOME to XDG_DATA_DIRS. XXX Why? Probably delete ...
# XXX export XDG_DATA_DIRS="$FLOX_DATA_HOME"${XDG_DATA_DIRS:+':'}${XDG_DATA_DIRS}

# Default profile "owner" directory, i.e. ~/.local/share/flox/environments/local/default/bin
declare defaultEnvironmentOwner="local" # as in "/usr/local"
if [ -L "$FLOX_ENVIRONMENTS/$defaultEnvironmentOwner" ]; then
	defaultEnvironmentOwner=$($_readlink "$FLOX_ENVIRONMENTS/$defaultEnvironmentOwner")
fi

# Path for floxmeta clone for current user (for access to floxmain).
declare userFloxMetaCloneDir="$FLOX_META/$defaultEnvironmentOwner"

# Define location for user-specific flox flake registry.
declare floxFlakeRegistry="$FLOX_CONFIG_HOME/floxFlakeRegistry.json"

# Manage user-specific nix.conf for use with flox only.
# XXX May need further consideration for Enterprise.
declare nixConf="$FLOX_CONFIG_HOME/nix.conf"
tmpNixConf="$($_mktemp --tmpdir="$FLOX_CONFIG_HOME")"
# We want the file in alphabetical order to ease comparing it.
# The consideration of access tokens is somewhat out of order.
# The remaining elements are appended below.
$_cat > "$tmpNixConf" <<EOF
# Automatically generated - do not edit.
accept-flake-config = true
connect-timeout = 5
EOF

# Ensure file is secure before appending access token(s).
${_chmod?} 600 "$tmpNixConf"

# Look for github tokens from multiple sources:
#   1. the user's own .config/nix/nix.conf, else
#   2. the user's gh client backing store, else
#   3. the user's own .config/flox/tokens (if it exists)
# ... and if found, extract and append tokens to .config/flox/nix.conf.
#
# We need to do this because this nix.conf file is the one [1] place
# where Nix will look to find access tokens for downloading URLs.
declare -a accessTokens=()
declare -A accessTokensMap # to detect/eliminate duplicates

if [ -f "$XDG_CONFIG_HOME/nix/nix.conf" ]; then
	for i in $($_awk '
		($1 == "access-tokens" && $2 == "=") {
			for (n=3; n<=NF; n++) {print $(n)}
		} ' "$HOME/.config/nix/nix.conf"); do
		if [ -z "${accessTokensMap[$i]}" ]; then
			accessTokens+=($i)
			accessTokensMap[$i]=1
		fi
	done
fi
if [ -f "$XDG_CONFIG_HOME/gh/hosts.yml" ]; then
	for i in $($_dasel -r yml -w json < "$XDG_CONFIG_HOME/gh/hosts.yml" | $_jq -r '(
			to_entries |
			map(select(.value.oauth_token != null)) |
			map("\(.key)=\(.value.oauth_token)") |
			join(" ")
		)'
	); do
		if [ -z "${accessTokensMap[$i]}" ]; then
			accessTokens+=($i)
			accessTokensMap[$i]=1
		fi
	done
fi
if [ -f "$FLOX_CONFIG_HOME/tokens" ]; then
	if [ "$($_stat -c %a $FLOX_CONFIG_HOME/tokens)" != "600" ]; then
		warn "fixing mode of $FLOX_CONFIG_HOME/tokens"
		$_chmod 600 "$FLOX_CONFIG_HOME/tokens"
	fi
	for i in $($_sed 's/#.*//' "$FLOX_CONFIG_HOME/tokens"); do
		# XXX add more syntax validation in golang rewrite
		if [ -z "${accessTokensMap[$i]}" ]; then
			accessTokens+=($i)
			accessTokensMap[$i]=1
		fi
	done
fi
# Append all available tokens to nix.conf.
if [[ "${#accessTokens[@]}" -gt 0 ]]; then
	echo "extra-access-tokens = ${accessTokens[@]}" >> $tmpNixConf
fi

# Add the remaining config values in alphabetical order
$_cat >> $tmpNixConf <<EOF
extra-experimental-features = nix-command flakes
extra-substituters = https://cache.floxdev.com
extra-trusted-public-keys = flox-store-public-0:8c/B+kjIaQ+BloCmNkRUKwaVPFWkriSAd0JJvuDu4F0=
flake-registry = $floxFlakeRegistry
netrc-file = $HOME/.netrc
warn-dirty = false
EOF

if $_cmp --quiet $tmpNixConf $nixConf; then
	$_rm $tmpNixConf
else
	warn "Updating \"$nixConf\""
	$_mv -f $tmpNixConf $nixConf
fi
export NIX_USER_CONF_FILES="$nixConf"
export SSL_CERT_FILE="${SSL_CERT_FILE:-@@NIXPKGS_CACERT_BUNDLE_CRT@@}"
export NIX_SSL_CERT_FILE="${NIX_SSL_CERT_FILE:-$SSL_CERT_FILE}"

# Similarly configure git config by way of $GIT_CONFIG_SYSTEM. Note that
# we do it by way of this env variable because Nix doesn't provide a
# passthru mechanism for passing options to git invocations. (?)
gitConfig="$FLOX_CONFIG_HOME/gitconfig"

tmpGitConfig=$($_mktemp --tmpdir=$FLOX_CONFIG_HOME)
$_chmod 600 $tmpGitConfig
$_cat > $tmpGitConfig <<EOF
# Automatically generated - do not edit.
[user]
	name = Flox User
	email = floxuser@example.invalid

# Use only https for accessing flox utility libraries.
[url "https://github.com/flox/capacitor"]
	insteadOf = "ssh://git@github.com/flox/capacitor"
	insteadOf = "git@github.com:flox/capacitor"

[url "https://github.com/flox/nixpkgs-flox"]
	insteadOf = "ssh://git@github.com/flox/nixpkgs-flox"
	insteadOf = "git@github.com:flox/nixpkgs-flox"

[url "https://github.com/flox/nixpkgs-catalog"]
	insteadOf = "ssh://git@github.com/flox/nixpkgs-catalog"
	insteadOf = "git@github.com:flox/nixpkgs-catalog"

[url "https://github.com/flox/catalog-ingest"]
	insteadOf = "ssh://git@github.com/flox/catalog-ingest"
	insteadOf = "git@github.com:flox/catalog-ingest"

[url "https://github.com/flox/flox-extras"]
	insteadOf = "ssh://git@github.com/flox/flox-extras"
	insteadOf = "git@github.com:flox/flox-extras"

[url "https://github.com/flox/bundlers"]
	insteadOf = "ssh://git@github.com/flox/bundlers"
	insteadOf = "git@github.com:flox/bundlers"

EOF
# XXX Remove after closed beta.

# Honor existing GIT_CONFIG_SYSTEM variable and/or default /etc/gitconfig.
if [ -n "${GIT_CONFIG_SYSTEM:-}" ]; then
	if [ -n "$FLOX_ORIGINAL_GIT_CONFIG_SYSTEM" ]; then
		# Reset GIT_CONFIG_SYSTEM to reflect the original value
		# observed before starting flox subshell (see below).
		GIT_CONFIG_SYSTEM="$FLOX_ORIGINAL_GIT_CONFIG_SYSTEM"
	fi
else
	if [ -e "/etc/gitconfig" ]; then
		GIT_CONFIG_SYSTEM="/etc/gitconfig"
	fi
fi

# If system gitconfig exists then include it, but check first to make sure
# user hasn't requested that we include our own gitconfig file(!).
if [[ -n "${GIT_CONFIG_SYSTEM:-}" ]] && [[ -e "$GIT_CONFIG_SYSTEM" ]] &&  \
   [[ "$GIT_CONFIG_SYSTEM" != "$gitConfig" ]]
then
	# Save first/original observed variable to disambiguate our use
	# of GIT_CONFIG_SYSTEM in subshells.
	export FLOX_ORIGINAL_GIT_CONFIG_SYSTEM="$GIT_CONFIG_SYSTEM"
	$_cat >> "$tmpGitConfig" <<EOF
[include]
	path = $GIT_CONFIG_SYSTEM

EOF
fi

# Compare generated gitconfig to cached version.
if $_cmp --quiet "$tmpGitConfig" "$gitConfig"; then
	$_rm "$tmpGitConfig"
else
	warn "Updating $gitConfig"
	$_mv -f "$tmpGitConfig" "$gitConfig"
fi

# Override system gitconfig.
export GIT_CONFIG_SYSTEM="$gitConfig"

if [ -n "${NIX_GET_COMPLETIONS:-}" ]; then
	export FLOX_ORIGINAL_NIX_GET_COMPLETIONS="$NIX_GET_COMPLETIONS"
	unset NIX_GET_COMPLETIONS
fi

# Load nix configuration (must happen after setting NIX_USER_CONF_FILES)
eval "$(nix_show_config)"

# Set FLOX_SYSTEM for this invocation. Be sure to inherit FLOX_SYSTEM
# from the environment if defined.
export FLOX_SYSTEM="${FLOX_SYSTEM:-$NIX_CONFIG_system}"
# Perform a quick sanity check of supported system types.
checkValidSystem "$FLOX_SYSTEM" ||
	error "invalid system type '$FLOX_SYSTEM'" </dev/null

# Save path to default env for convenience throughout.
declare defaultEnv="$FLOX_ENVIRONMENTS/$defaultEnvironmentOwner/$FLOX_SYSTEM.default"

# Load configuration from [potentially multiple] flox.toml config file(s).
# Note: not using this data for anything yet but keeping it here as
# placeholder for functionality. Expect it to figure prominently in
# tenant customizations.
eval "$(read_flox_conf git_base_url)"
if [ -z "${FLOX_CONF_git_base_url:-}" ]; then
	# attempt to read old bash floxpkgs.gitBaseURL value from old flox.toml
	eval "$(read_flox_conf floxpkgs)"
	if [ -n "${FLOX_CONF_floxpkgs:-}" ]; then
		FLOX_CONF_git_base_url="$($_jq -r -n --argjson floxpkgs "$FLOX_CONF_floxpkgs" '$floxpkgs["gitBaseURL"]')"
	else
		warn "could not read git_base_url from config; defaulting to https://github.com/"
		FLOX_CONF_git_base_url="https://github.com/"
	fi
fi

# Bootstrap user-specific configuration.
. "$_lib/bootstrap.sh"

# Populate user-specific flake registry.
declare -A validChannels=()
#shellcheck disable=SC2119
updateFloxFlakeRegistry

# Leave it to Bob to figure out that Nix 2.3 has the bug that it invokes
# `tar` without the `-f` flag and will therefore honor the `TAPE` variable
# over STDIN (to reproduce, try running `TAPE=none flox shell`).
# XXX Still needed??? Probably delete ...
unset TAPE

# Timestamp
now="$($_date +%s)"

# vim:ts=4:noet:syntax=bash
