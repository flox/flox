#!/usr/bin/env bash
#
# flox.sh - Flox CLI
#

# Ensure that the script dies on any error.
set -e
set -o pipefail

# Declare default values for debugging variables.
declare -i verbose=0
declare -i debug=0

# Declare global variables
declare -i educatePublish=0
declare -i interactive=0
declare -i spawnMode=0

# set -x if debugging, can never remember which way this goes so do both.
# Note need to do this here in addition to "-d" flag to be able to debug
# initial argument parsing.
test -z "${DEBUG_FLOX}" || FLOX_DEBUG="${DEBUG_FLOX}"
test -z "${FLOX_DEBUG}" || set -x

# Similar for verbose.
test -z "${FLOX_VERBOSE}" || verbose=1

# Import configuration, load utility functions, etc.
_prefix="@@PREFIX@@"
_prefix=${_prefix:-.}
_lib=$_prefix/lib
_etc=$_prefix/etc
_share=$_prefix/share

# If the first arguments are any of -d|--date, -v|--verbose or --debug
# then we consume this (and in the case of --date, its argument) as
# argument(s) to the wrapper and not the command to be wrapped. To send
# either of these arguments to the wrapped command put them at the end.
while [ $# -ne 0 ]; do
	case "$1" in
	--stability)
		shift
		if [ $# -eq 0 ]; then
			echo "ERROR: missing argument to --stability flag" 1>&2
			exit 1
		fi
		export FLOX_STABILITY="$1"
		shift
		;;
	--system)
		shift
		if [ $# -eq 0 ]; then
			echo "ERROR: missing argument to --system flag" 1>&2
			exit 1
		fi
		# Validation happens later in lib/init.sh.
		export FLOX_SYSTEM="$1"
		shift
		;;
	-d | --date)
		shift
		if [ $# -eq 0 ]; then
			error "missing argument to --date flag" </dev/null
		fi
		export FLOX_RENIX_DATE="$1"
		shift
		;;
	-v | --verbose)
		(( ++verbose ))
		shift
		;;
	--debug)
		(( ++debug ))
		[ $debug -le 1 ] || set -x
		(( ++verbose ))
		shift
		;;
	--prefix)
		echo "$_prefix"
		exit 0
		;;
	-V | --version)
		echo "Version: @@VERSION@@"
		exit 0
		;;
	-h | --help)
		# Perform initialization to pull in usage().
		. $_lib/init.sh
		usage
		exit 0
		;;
	*) break ;;
	esac
done

# Save the original invocation string.
declare invocation_string="$0 $*"

# Perform initialization with benefit of flox CLI args set above.
. $_lib/init.sh

# Improve upon the invocation string now we have pprint().
invocation_string=$(pprint "$me" "$@")

#
# main()
#

# Start by identifying subcommand to be invoked.
# FIXME: use getopts to properly scan args for first non-option arg.
while test $# -gt 0; do
	case "$1" in
	-*)
		error "unrecognised option before subcommand" </dev/null
		;;
	*)
		subcommand="$1"
		shift
		break
		;;
	esac
done
if [ -z "$subcommand" ]; then
	usage | error "command not provided"
fi

# Flox aliases
if [ "$subcommand" = "rm" ] || [ "$subcommand" = "uninstall" ]; then
       subcommand=remove
fi

# Store the original subcommand invocation arguments.
declare -a invocation_args=("$@")

# Flox environment path(s).
declare -a environments=()

# Build log message as we go.
logMessage=

case "$subcommand" in

# Flox commands which take an (-e|--environment) environment argument.
activate | history | create | install | list | remove | rollback | \
	switch-generation | upgrade | \
	import | export | edit | generations | git | push | pull | destroy)

	# Look for the --environment and --system argument(s).
	args=()
	while test $# -gt 0; do
		case "$1" in
		-e | --environment)
			environments+=("$2")
			shift 2
			;;
		--system)
			# Perform a quick sanity check of supported system types.
			shift
			checkValidSystem "$1" ||
				error "invalid system type '$1'" </dev/null
			export FLOX_SYSTEM="$1"
			shift
			;;
		--)
			args+=("$@")
			break
			;;
		*)
			args+=("$1")
			shift
			;;
		esac
	done
	if [ ${#environments[@]} -eq 0 ]; then
		environments+=("$(selectDefaultEnvironment "$subcommand" "$defaultEnv")")
	else
		declare -a environmentArgs=()
		for i in "${environments[@]}"; do
			environmentArgs+=("$(environmentArg "$i")")
		done
		environments=("${environmentArgs[@]}")
	fi

	# Only the "activate" subcommand accepts multiple environments.
	if [ "$subcommand" != "activate" -a ${#environments[@]} -gt 1 ]; then
		usage | error "\"$subcommand\" does not accept multiple -e|--environment arguments"
	fi

	environment=${environments[0]}

	# project environments are only valid for a subset of subcommands.
	if [[ "$environment" =~ '#' ]]; then # project flox environment
		case "$subcommand" in
		activate|edit|install|list|remove|upgrade) # support project environments
			: ;; # pass
		*) # all other commands do not support project environments
			error "'$subcommand' not supported for project environments" < /dev/null
			;;
		esac
	fi

	[ $verbose -eq 0 ] || [ "$subcommand" = "activate" ] || echo Using environment: $environment >&2

	case "$subcommand" in

	## Environment commands
	# Reminder: "${args[@]}" has the -e and --system args removed.
	activate)
		floxActivate "${environments[*]}" "$FLOX_SYSTEM" "${args[@]}";;
	create)
		floxCreate "$environment" "$FLOX_SYSTEM" "${args[@]}";;
	destroy)
		floxDestroy "$environment" "$FLOX_SYSTEM" "${args[@]}";;
	edit)
		floxEdit "$environment" "$FLOX_SYSTEM" "${args[@]}";;
	export)
		floxExport "$environment" "$FLOX_SYSTEM" "${args[@]}";;
	generations)
		floxGenerations "$environment" "$FLOX_SYSTEM" "${args[@]}";;
	git)
		floxGit "$environment" "${args[@]}";;
	history)
		floxHistory "$environment" "$FLOX_SYSTEM" "${args[@]}";;
	import)
		floxImport "$environment" "$FLOX_SYSTEM" "${args[@]}";;
	install)
		floxInstall "$environment" "$FLOX_SYSTEM" "${args[@]}";;
	list)
		floxList "$environment" "$FLOX_SYSTEM" "${args[@]}";;
	push | pull)
		floxPushPull "$subcommand" "$environment" "$FLOX_SYSTEM" "${args[@]}";;
	remove)
		floxRemove "$environment" "$FLOX_SYSTEM" "${args[@]}";;
	rollback|switch-generation)
		if [ "$subcommand" = "switch-generation" ]; then
			# rewrite switch-generation to instead use the new
			# "rollback --to" command (which makes no sense IMO).
			args=("--to" "${args[@]}")
		fi
		floxRollback "$environment" "$FLOX_SYSTEM" $subcommand "${args[@]}";;
	upgrade)
		floxUpgrade "$environment" "$FLOX_SYSTEM" "${args[@]}";;
	*)
		usage | error "Unknown command: $subcommand"
		;;

	esac
	;;

# Flox commands which derive an attribute path from the current directory.
build | develop | print-dev-env | eval | publish | run | shell)
	case "$subcommand" in
	build)
		floxBuild "$@"
		;;
	develop|print-dev-env)
		if [ "$subcommand" = "develop" -a $interactive -eq 0 ]; then
			usage | error "'flox develop' must be invoked interactively"
		fi
		if [ "$subcommand" = "print-dev-env" ]; then
			# Force non-interactive mode
			interactive=0
			# Also make sure to print bash syntax compatible with that
			# coming out of 'nix develop', regardless of what shell the
			# user may be using.
			export SHELL=bash
		fi
		floxDevelop "$@"
		;;
	eval)
		floxEval "$@"
		;;
	publish)
		floxPublish "$@"
		;;
	run)
		floxRun "$@"
		;;
	shell)
		floxShell "$@"
		;;
	esac
	;;

# The environments subcommand takes no arguments.
envs | environments)
	floxEnvironments "$FLOX_SYSTEM" "${invocation_args[@]}"
	;;

gh)
	verboseExec $_flox_gh "$@"
	;;

init)
	floxInit "$@"
	;;

packages|search)
	floxSearch "$@"
	;;

# Special "cut-thru" mode to invoke Nix directly.
nix)
	if [ -n "$FLOX_ORIGINAL_NIX_GET_COMPLETIONS" ]; then
		export NIX_GET_COMPLETIONS="$(( FLOX_ORIGINAL_NIX_GET_COMPLETIONS - 1 ))"
	fi
	verboseExec $_nix "$@"
	;;

config)
	floxConfig "${invocation_args[@]}"
	;;

subscribe)
	floxSubscribe "${invocation_args[@]}"
	;;

unsubscribe)
	floxUnsubscribe "${invocation_args[@]}"
	;;

channels)
	floxChannels "${invocation_args[@]}"
	;;

help)
	# Believe it or not the man package relies on finding both "cat" and
	# "less" in its PATH, and even when we patch the man package it then
	# calls "nroff" (in the groff package) which is similarly broken.
	# So, for this one instance just add coreutils & less to the PATH.
	export PATH="@@FLOXPATH@@"
	verboseExec $_man -l "$_share/man/man1/flox.1.gz"
	;;

*)
	verboseExec $_nix "$subcommand" "$@"
	;;

esac

# vim:ts=4:noet:syntax=bash
