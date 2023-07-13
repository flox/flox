# -*- mode: sh; sh-shell: bash; -*-
## Environment commands

#
# bashRC($@)
#
# Takes a list of environments and emits bash commands to configure each
# of them in the order provided.
#
function bashRC() {
	trace "$@"
	# Start with required platform-specific Nixpkgs environment variables.
	$_grep -v '^#' "$_lib/commands/shells/activate.bash" | $_grep -v '^$'
	# Add computed environment variables.
	for i in PATH XDG_DATA_DIRS FLOX_ACTIVE_ENVIRONMENTS                    \
			FLOX_PROMPT_ENVIRONMENTS FLOX_PROMPT_COLOR_{1,2}; do
		printf 'export %s="%s"\n' "$i" "${!i}"
	done
	# Add environment-specific activation commands.
	for i in "$@"; do
		# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
		eval "$(decodeEnvironment "$i")"
		echo "export FLOX_ENV='$environmentBaseDir'"
		if [ -f "$environmentBaseDir/activate" ]; then
			$invoke_cat "$environmentBaseDir/activate"
		elif [ -f "$environmentBaseDir/manifest.toml" ]; then
			# Original v1 format to be deprecated.
			(metaGitShow "$environmentBaseDir" manifest.toml 2>/dev/null | manifestTOML bashInit) || :
		fi
	done
}

#
# Regardless of the context in which "flox activate" is invoked it does
# three things, although it may not do all of these in every context:
#
#   I. sets environment variables
#   II. runs hooks
#   III. invokes a _single_ command
#
# ... and "flox activate" can be invoked in the following contexts:
#
#   A. with arguments denoting a command to be invoked
#      1. creates an "rc" script in bash (i.e. flox CLI shell)
#      2. if NOT environment already active
#        - appends commands (I && II) to the "rc" script
#      3. source "rc" script directly and exec() $cmdArgs (III)
#   B. in an interactive context
#      1. creates an "rc" script in the language of the user's $SHELL
#      2. if NOT environment already active
#        - appends commands (I && II) to the "rc" script
#      3. exec() $SHELL (III) with "rc" configured to source script
#   C. in a non-interactive context
#      0. confirms the running shell (cannot trust $SHELL)
#      1. creates an "rc" script in the language of the running shell
#      2. if NOT environment already active
#        - appends commands (I && II) to the "rc" script
#      3. cat() contents of "rc" script to stdout (does not invoke anything)
#      4. remove "rc" script
#
# Breaking it down in this way allows us to employ common logic across
# all cases. In the B and C cases we take over the shell "rc" entrypoint
# so that we can guarantee that flox environment directories are prepended
# to the PATH *AFTER* all other processing has been completed. This is
# particularly important in the case of Darwin which has a "path_helper"
# that re-orders the PATH in a decidedly "unhelpful" way with each new
# shell invocation.
#

_environment_commands+=("activate")
_usage["activate"]="activate environment:
        in current shell: eval \"\$(flox activate)\"
        in subshell: flox activate
        for command: flox activate -- <command> <args>"

function floxActivate() {
	trace "$@"
	local -a environments
	read -ra environments <<< "$1"; shift
	local _target_environment="${environments[0]}"
	local system="$1"; shift
	local -a invocation=("$@")
	local -A _flox_active_environments_hash
	local -a _flox_original_active_environments_array
	local -a _environments_requested
	local -a _environments_to_activate

	local -a cmdArgs=()
	local -i inCmdArgs=0

	while [[ "$#" -gt 0 ]]; do
		case "$1" in
		# User has explicitly requested a system which differs from the
		# running system.
		# This is largely useful for `aarch64-darwin' systems which have the
		# ability to execute `x86_64-darwin' binaries.
		-s|--system)
			shift;
			if [[ "$#" -lt 1 ]]; then
				error "option \`--system <SYSTEM>' requires an argument"
			fi
			system="$1"
			;;
		--)
			if [[ "$inCmdArgs" -eq 1 ]]; then
				cmdArgs+=("$1")
			else
				inCmdArgs=1
			fi
			;;
		*)
			if [[ "$inCmdArgs" -eq 1 ]]; then
				cmdArgs+=("$1")
			else
				usage | error "unexpected argument \"$1\" passed to \"$subcommand\""
			fi
			;;
		esac
		shift;
	done

	# The $FLOX_ACTIVE_ENVIRONMENTS variable is colon-separated (like $PATH)
	# and contains the list of fully-qualified active environments by path,
	# e.g. /Users/floxfan/.local/share/flox/environments/local/default.
	# Load this variable into an associative array for convenient lookup.
	IFS=: read -ra _flox_original_active_environments_array  \
	               <<< "$FLOX_ACTIVE_ENVIRONMENTS"
	for i in "${_flox_original_active_environments_array[@]}"; do
		_flox_active_environments_hash["$i"]=1
	done

	# Identify each environment requested, taking note of all those that
	# have not yet been activated so we can be sure to avoid running their
	# activation scripts multiple times.
	for i in "${environments[@]}"; do
		_environments_requested+=("$i")
		if [ -z "${_flox_active_environments_hash[$i]}" ]; then
			# Only warn if not a project or the default environment.
			if [[ "$i" != "$defaultEnv" && ! "$i" =~ '#' ]]; then
				[ -d "$i/." ] || warn "INFO environment not found: $i"
			fi
			_environments_to_activate+=("$i")
			_flox_active_environments_hash["$i"]=1
		elif [ "$i" != "$defaultEnv" ]; then
			# Only warn if in an interactive session, and don't warn when
			# attempting to activate the default env.
			if [ "$interactive" -eq 1 ]; then
				warn "INFO not running hooks for active environment: $i"
			fi
		fi
	done

	# Add "default" to end of the list if it's not already there.
	# Do this separately from loop above to detect when people
	# explicitly attempt to activate default env twice.
	if [ -z "${_flox_active_environments_hash[$defaultEnv]}" ]; then
		_environments_requested+=("$defaultEnv")
		_environments_to_activate+=("$defaultEnv")
		_flox_active_environments_hash["$defaultEnv"]=1
	fi

	# filter out any project environments for update operations
	local -a floxmeta_environments
	for environment in "${_environments_to_activate[@]}"                     \
	                   "${_flox_original_active_environments_array[@]}"; do
		if [[ ! "$environment" =~ '#' ]]; then
			floxmeta_environments+=("$environment")
		fi
	done
	# Before possibly bailing out, check to see if any of the active or
	# about-to-be-activated environments have updates pending.
	for environment in "${floxmeta_environments[@]}"; do
		local -i autoUpdate
		autoUpdate="$(doAutoUpdate "$environment")"
		if [ "$autoUpdate" -ne 0 ]; then
			local -i updateGen
			updateGen="$(updateAvailable "$environment")"
			if [ "$updateGen" -gt 0 ]; then
				if [ "$autoUpdate" -eq 1 ]; then
					# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
					eval "$(decodeEnvironment "$environment")"
					if $_gum confirm "'$environmentAlias' is at generation $updateGen, pull latest version?"; then
						floxPushPull pull "$environment" "$system"
					fi
				else # $autoUpdate == 2, aka always pull without prompting
					floxPushPull pull "$environment" "$system"
				fi
			fi
		fi
	done
	trailingAsyncFetch "${floxmeta_environments[@]}"

	# Determine shell language to be used for "rc" script.
	local rcShell
	if [ "${#cmdArgs[@]}" -gt 0 ]; then
		rcShell="$_bash" # i.e. language of this script
	elif [ "$spawnMode" -eq 1 ]; then
		# "Spawn" mode. Configure environment using bash then exec $rcShell.
		rcShell="$SHELL" # i.e. the shell we will be invoking
	else
		# "Source" mode. In this case it's really important to emit commands
		# using the correct syntax, so start by doing everything possible to
		# accurately identify the currently-running (parent) shell.
		rcShell="$(identifyParentShell)";
		# Just in case we got it wrong, only trust $rcShell if it "smells like
		# a shell", which AFAIK is best expressed as ending in "sh".
		case "$rcShell" in
		*sh) : ;;
		*) # Weird ... this warrants a warning.
			warn "WARNING: calling process '$rcShell' does not look like a shell .. using '$SHELL' syntax"
			rcShell="$SHELL"
			;;
		esac
	fi

	# Build up strings to be prepended to environment variables.
	# Note the requirement to prepend in the order provided, e.g.
	# if activating environments 'A' and 'B' in that order then
	# the string to be prepended to PATH is 'A/bin:B/bin'.
	#
	# Note that we set variables for all environments requested,
	# regardless if they have already been activated, so that people
	# can a) re-order their environments after activation and b) to
	# prevent `flox activate` invocations from failing unnecessarily.
	local -a path_prepend=()
	local -a xdg_data_dirs_prepend=()
	local -a flox_active_environments_prepend=()
	local -a flox_prompt_environments_prepend=()
	for environment in "${_environments_to_activate[@]}"; do
		# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
		eval "$(decodeEnvironment "$environment")"
		path_prepend+=("$environmentBinDir")
		xdg_data_dirs_prepend+=("$environmentBaseDir/share")
		flox_active_environments_prepend+=("$environment")
		flox_prompt_environments_prepend+=("$environmentAlias")
	done
	PATH="$(joinString ':' "${path_prepend[@]}" "$PATH")"
	XDG_DATA_DIRS="$(joinString ':' "${xdg_data_dirs_prepend[@]}" "$XDG_DATA_DIRS")"
	FLOX_ACTIVE_ENVIRONMENTS="$(joinString ':' "${flox_active_environments_prepend[@]}" "$FLOX_ACTIVE_ENVIRONMENTS")"
	FLOX_PROMPT_ENVIRONMENTS="$(joinString ' ' "${flox_prompt_environments_prepend[@]}" "$FLOX_PROMPT_ENVIRONMENTS")"
	export PATH XDG_DATA_DIRS FLOX_ACTIVE_ENVIRONMENTS FLOX_PROMPT_ENVIRONMENTS

	# Darwin has a "path_helper" which indiscriminately reorders the path to
	# put the Apple-preferred items first in the PATH, which completely breaks
	# the user's ability to manage their PATH in subshells, e.g. when using tmux.
	#
	# Trouble is, there's really no way to undo the damage done by the "path_helper"
	# apart from inflicting the similarly heinous approach of again reordering the
	# PATH to put flox environments at the front. It's fighting fire with fire, but
	# unless we want to risk even further breakage by disabling path_helper in
	# /etc/zprofile this is the best workaround we've come up with.
	#
	# https://discourse.floxdev.com/t/losing-part-of-my-shell-environment-when-using-flox-develop/556/2
	if [[ -x /usr/libexec/path_helper ]] && [[ "$PATH" =~ ^/usr/local/bin: ]]
	then
		if [[ "${#cmdArgs[@]}" -eq 0 ]] && [[ "$spawnMode" -eq 0 ]]; then
			case "$rcShell" in
			*bash|*zsh)
				PATH="$(echo "$PATH" | $_awk -v shellDialect=bash -f "$_libexec/flox/darwin-path-fixer.awk")"
				export PATH
				;;
			esac
		fi
	fi

	# Create "rc" script.
	local rcScript
	rcScript="$(mktemp)" # cleans up after itself, do not use mkTempFile()
	case "$rcShell" in
	*bash|*dash)
		bashRC "${_environments_to_activate[@]}" >> "$rcScript"
		;;
	*zsh)
		# The zsh fpath variable must be prepended with each new subshell.
		local -a fpath_prepend=()
		for i in $(joinString ' ' "${_environments_requested[@]}" "${_flox_original_active_environments_array[@]}"); do
			# Add to fpath irrespective of whether the directory exists at
			# activation time because people can install to an environment
			# while it is active and immediately benefit from commandline
			# completion.
			eval "$( decodeEnvironment "$i"; )";
			fpath_prepend+=(
				"$environmentBaseDir/share/zsh/site-functions"
				"$environmentBaseDir/share/zsh/vendor-completions"
			)
		done
		if [ "${#fpath_prepend[@]}" -gt 0 ]; then
			{
			  printf "fpath=("
			  printf "'%s' " "${fpath_prepend[@]}"
			  echo "\$fpath)"
			  echo "autoload -U compinit && compinit"
			} >> "$rcScript"
		fi
		bashRC "${_environments_to_activate[@]}" >> "$rcScript"
		;;
	*csh|*fish)
		error "unsupported shell: $rcShell" < /dev/null
		;;
	*)
		error "unknown shell: $rcShell" < /dev/null
		;;
	esac

	# Earlier parts of the activation script set individual `FLOX_ENV' vars
	# interspersed between `shell.hook' bodies.
	# Because the "target" environment may not be the last to activate, we
	# explicitly set `FLOX_ENV' here so that once activated the variable will
	# point to the first environment indicated on the command line.
	eval "$(decodeEnvironment "$_target_environment")"
	echo "export FLOX_ENV='$environmentBaseDir'" >> "$rcScript"

	# Set the init script to self-destruct upon activation (unless debugging).
	# Very James Bond.
	[ "$debug" -gt 0 ] || echo "$_rm $rcScript" >> "$rcScript"

	# If invoking a command, go ahead and exec().
	if [ "${#cmdArgs[@]}" -gt 0 ]; then
		# Command case - source "rc" script and exec command.
		source "$rcScript"
		[ "$verbose" -eq 0 ] || pprint "+$colorBold" exec "${cmdArgs[@]}" "$colorReset" 1>&2
		exec "${cmdArgs[@]}" # Does not return.
	fi

	# Add commands to configure prompt for interactive shells. The
	# challenge here is that this code can be called one or two
	# times for a single activation, i.e. person can do one or
	# both of the following:
	#
	# - invoke 'flox activate -e foo'
	# - have 'eval "$(flox activate)"' in .zshrc
	#
	# Our only real defense against this sort of "double activation"
	# is to put guards around our configuration, just as C include
	# files have had since the dawn of time.
	if [ -z "$FLOX_PROMPT_DISABLE" ]; then
		case "$rcShell" in
		*bash)
			cat "$_etc/flox.prompt.bashrc" >> "$rcScript"
			;;
		*zsh)
			cat "$_etc/flox.zdotdir/prompt.zshrc" >> "$rcScript"
			;;
		esac
	fi

	# Address possibility of corrupt /etc/zshrc* files on Darwin.
	[ "$($_uname -s)" != "Darwin" ] || darwinRepairFiles

	# Activate.
	if [ "$spawnMode" -eq 1 ]; then
		# Spawn mode - launch subshell.
		case "$rcShell" in
		*bash|*dash)
			export FLOX_BASH_INIT_SCRIPT="$rcScript"
			[ "$verbose" -eq 0 ] || pprint "+$colorBold" exec "$rcShell" "--rcfile" "$_etc/flox.bashrc" "$colorReset" 1>&2
			case "$rcShell" in
				*bash) exec "$rcShell" "--rcfile" "$_etc/flox.bashrc"; ;;
				# `dash' lacks an equivalent for `--rcfile' so we have to do
				# things "the good ol' fashioned way" - manually sourcing the
				# profile script and then executing an interactive shell.
				*dash)
					exec "$rcShell" -c                                      \
					       "source '$_etc/flox.bashrc'; exec $rcShell -i";
					;;
			esac
			;;
		*zsh)
			export FLOX_ZSH_INIT_SCRIPT="$rcScript"
			if [ -n "$ZDOTDIR" ]; then
				[ "$verbose" -eq 0 ] || warn "+ export FLOX_ORIG_ZDOTDIR=\"$ZDOTDIR\""
				export FLOX_ORIG_ZDOTDIR="$ZDOTDIR"
			fi
			[ "$verbose" -eq 0 ] || warn "+ export ZDOTDIR=\"$_etc/flox.zdotdir\""
			export ZDOTDIR="$_etc/flox.zdotdir"
			[ "$verbose" -eq 0 ] || pprint "+$colorBold" exec "$rcShell" "$colorReset" 1>&2
			exec "$rcShell"
			;;
		*)
			warn "unsupported shell: \"$rcShell\""
			warn "Launching bash instead"
			[ "$verbose" -eq 0 ] || pprint "+$colorBold" exec "$rcShell" "--rcfile" "$_etc/flox.bashrc" "$colorReset" 1>&2
			exec "$rcShell" "--rcfile" "$_etc/flox.bashrc"
			;;
		esac
	else
		# Source mode - print out commands to be sourced.
		local _flox_activate_verbose=/dev/null
		[ "$verbose" -eq 0 ] || _flox_activate_verbose=/dev/stderr
		case "$rcShell" in
		*bash|*zsh|*dash)
			$_cat "$rcScript" | $_tee "$_flox_activate_verbose"
			;;
		*)
			error "unsupported shell: \"$rcShell\" - please run 'flox activate' in interactive mode" </dev/null
			;;
		esac
	fi
}

# vim:ts=4:noet:syntax=bash
