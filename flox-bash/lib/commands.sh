#
# lib/commands.sh: one function for each subcommand
#
# The design of this library is that common options are first parsed
# in flox.sh, then any command-specific handling is performed from
# within functions in this file.
#
# * functions use local variable declarations wherever possible
# * functions in this file are sorted alphabetically (usage will match!)
# * functions are named "floxCommand" to match the corresponding command
# * _usage* variables are mandatory, defined immediately prior to functions
#   - usage sections (not sorted): general, environment, development
# * functions return a command array to be invoked in the calling function
# named "floxCommand" to match the corresponding command
# * cargo cult: use tabs, comments, formatting, etc. to match existing examples
#

# This first function sorts the provided options and arguments into
# those supported by the top level `nix` command as opposed to its
# various subcommands, as parsed in each of the functions below this
# one. It employs global variables because it's not easy to return
# two output streams with bash.
declare -a _nixArgs
declare -a _cmdArgs
function parseNixArgs() {
	_nixArgs=()
	_cmdArgs=()
	while test $# -gt 0; do
		case "$1" in
		# Options taking two args.
		--option)
			_nixArgs+=("$1"); shift
			_nixArgs+=("$1"); shift
			_nixArgs+=("$1"); shift
			;;
		# Options taking one arg.
		--access-tokens | --allowed-impure-host-deps | --allowed-uris | \
		--allowed-users | --bash-prompt | --bash-prompt-prefix | \
		--bash-prompt-suffix | --build-hook | --build-poll-interval | \
		--build-users-group | --builders | --commit-lockfile-summary | \
		--connect-timeout | --cores | --diff-hook | \
		--download-attempts | --experimental-features | --extra-access-tokens | \
		--extra-allowed-impure-host-deps | --extra-allowed-uris | --extra-allowed-users | \
		--extra-experimental-features | --extra-extra-platforms | --extra-hashed-mirrors | \
		--extra-ignored-acls | --extra-nix-path | --extra-platforms | \
		--extra-plugin-files | --extra-sandbox-paths | --extra-secret-key-files | \
		--extra-substituters | --extra-system-features | --extra-trusted-public-keys | \
		--extra-trusted-substituters | --extra-trusted-users | --flake-registry | \
		--gc-reserved-space | --hashed-mirrors | --http-connections | \
		--ignored-acls | --log-format |--log-lines | \
		--max-build-log-size | --max-free | --max-jobs | \
		--max-silent-time | --min-free | --min-free-check-interval | \
		--nar-buffer-size | --narinfo-cache-negative-ttl | --narinfo-cache-positive-ttl | \
		--netrc-file | --nix-path | --plugin-files | \
		--post-build-hook | --pre-build-hook | --repeat | \
		--sandbox-build-dir | --sandbox-dev-shm-size | --sandbox-paths | \
		--secret-key-files | --stalled-download-timeout | --store | \
		--substituters | --system | --system-features | \
		--tarball-ttl | --timeout | --trusted-public-keys | \
		--trusted-substituters | --trusted-users | --user-agent-suffix)
			_nixArgs+=("$1"); shift
			_nixArgs+=("$1"); shift
			;;
		# Options taking zero args.
		--help | --offline | --refresh | --version | --debug | \
		--print-build-logs | -L | --quiet | --verbose | -v | \
		--accept-flake-config | --allow-dirty | --allow-import-from-derivation | \
		--allow-new-privileges | --allow-symlinked-store | \
		--allow-unsafe-native-code-during-evaluation | --auto-optimise-store | \
		--builders-use-substitutes | --compress-build-log | \
		--enforce-determinism | --eval-cache | --fallback | --filter-syscalls | \
		--fsync-metadata | --http2 | --ignore-try | --impersonate-linux-26 | \
		--keep-build-log | --keep-derivations | --keep-env-derivations | \
		--keep-failed | --keep-going | --keep-outputs | \
		--no-accept-flake-config | --no-allow-dirty | \
		--no-allow-import-from-derivation | --no-allow-new-privileges | \
		--no-allow-symlinked-store | \
		--no-allow-unsafe-native-code-during-evaluation | \
		--no-auto-optimise-store | --no-builders-use-substitutes | \
		--no-compress-build-log | --no-enforce-determinism | --no-eval-cache | \
		--no-fallback | --no-filter-syscalls | --no-fsync-metadata | \
		--no-http2 | --no-ignore-try | --no-impersonate-linux-26 | \
		--no-keep-build-log | --no-keep-derivations | --no-keep-env-derivations | \
		--no-keep-failed | --no-keep-going | --no-keep-outputs | \
		--no-preallocate-contents | --no-print-missing | --no-pure-eval | \
		--no-require-sigs | --no-restrict-eval | --no-run-diff-hook | \
		--no-sandbox | --no-sandbox-fallback | --no-show-trace | \
		--no-substitute | --no-sync-before-registering | \
		--no-trace-function-calls | --no-trace-verbose | --no-use-case-hack | \
		--no-use-registries | --no-use-sqlite-wal | --no-warn-dirty | \
		--preallocate-contents | --print-missing | --pure-eval | \
		--relaxed-sandbox | --require-sigs | --restrict-eval | --run-diff-hook | \
		--sandbox | --sandbox-fallback | --show-trace | --substitute | \
		--sync-before-registering | --trace-function-calls | --trace-verbose | \
		--use-case-hack | --use-registries | --use-sqlite-wal | --warn-dirty)
			_nixArgs+=("$1"); shift
			;;
		# Consume remaining args
		--command|-c|--)
			_cmdArgs+=("$@")
			break
			;;
		# All else are command args.
		*)
			_cmdArgs+=("$1"); shift
			;;
		esac
	done
}

# Import command functions. N.B. the order of imports determines the
# order that commands appear in the usage() statement.

## General commands
. $_lib/commands/general.sh

## Environment commands
. $_lib/commands/activate.sh
. $_lib/commands/environment.sh

## Development commands
. $_lib/commands/development.sh
. $_lib/commands/publish.sh

# vim:ts=4:noet:syntax=bash
