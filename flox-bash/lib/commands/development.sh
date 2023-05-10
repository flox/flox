## Development commands

# flox init
_development_commands+=("init")
_usage["init"]="initialize flox expressions for current project"
function floxInit() {
	trace "$@"
	parseNixArgs "$@" && set -- "${_cmdArgs[@]}"

	local template
	local pname
	while test $# -gt 0; do
		case "$1" in
		-t | --template) # takes one arg
			shift
			template="$1"
			shift
			;;
		-n | --name) # takes one arg
			shift
			pname="$1"
			shift
			;;
		*)
			usage | error "invalid argument: $1"
			shift
			;;
		esac

	done

	# Select template.
	if [[ -z "$template" ]]; then
		template=$($_nix eval --no-write-lock-file --raw --apply '
		  x: with builtins; concatStringsSep "\n" (
			attrValues (mapAttrs (k: v: k + ": " + v.description) (removeAttrs x ["_init"]))
		  )
		' "flox#templates" | $_gum filter | $_cut -d: -f1)
		[ -n "$template" ] || exit 1
	fi

	# Identify pname.
	if [[ -z "$pname" ]]; then
		local origin
		origin=$($_git remote get-url origin)
		local bn=${origin//*\//}
		local pname
		pname=$($_gum input --value "${bn//.git/}" --prompt "Enter package name: ")
		[ -n "$pname" ] || exit 1
	fi

	# Extract flox _init template if it hasn't already.
	[ -f flox.nix ] || {
		# Start by extracting "_init" template to floxify project.
		$invoke_nix flake init --template "flox#templates._init" "$@"
	}

	# Extract requested template.
	$invoke_nix "${_nixArgs[@]}" flake init --template "flox#templates.$template" "$@"
	if [ -f pkgs/default.nix ]; then
		$invoke_mkdir -p "pkgs/$pname"
		$invoke_git mv pkgs/default.nix "pkgs/$pname/default.nix"
		echo "renamed: pkgs/default.nix -> pkgs/$pname/default.nix" 1>&2
		$invoke_sed -i -e \
			"s/pname = \".*\";/pname = \"$pname\";/" \
			"pkgs/$pname/default.nix"
	fi
}

# flox build
_development_commands+=("build")
_usage["build"]="build package from current project"
function floxBuild() {
	trace "$@"
	parseNixArgs "$@" && set -- "${_cmdArgs[@]}"

	local -a buildArgs=()
	local -a installables=()
	while test $# -gt 0; do
		case "$1" in
		-A | --attr) # takes one arg
			# legacy nix-build option; convert to flakeref
			shift
			installables+=(".#$1"); shift
			;;

		# All remaining options are `nix build` args.

		# Options taking two args.
		-o|--profile|--override-flake|--override-input)
			buildArgs+=("$1"); shift
			buildArgs+=("$1"); shift
			buildArgs+=("$1"); shift
			;;
		# Options taking one arg.
		--out-link|--eval-store|--include|-I|--inputs-from|--update-input|--expr|--file|-f)
			buildArgs+=("$1"); shift
			buildArgs+=("$1"); shift
			;;
		# Options taking zero args.
		-*)
			buildArgs+=("$1"); shift
			;;
		# Assume all other options are installables.
		*)
			installables+=("$1"); shift
			;;
		esac

	done

	# If no installables specified then try identifying attrPath from
	# capacitated flake.
	if [ ${#installables[@]} -eq 0 ]; then
		local attrPath="$(selectAttrPath . build packages)"
		installables=(".#$attrPath")
	fi

	if [ -n "$FLOX_ORIGINAL_NIX_GET_COMPLETIONS" ]; then
		export NIX_GET_COMPLETIONS="$(( FLOX_ORIGINAL_NIX_GET_COMPLETIONS + 1 ))"
	fi
	$invoke_nix "${_nixArgs[@]}" build --impure "${buildArgs[@]}" "${installables[@]}" --override-input flox-floxpkgs/nixpkgs/nixpkgs flake:nixpkgs-$FLOX_STABILITY
}

# flox eval
_development_commands+=("eval")
_usage["eval"]="evaluate a Nix expression"
function floxEval() {
	trace "$@"
	parseNixArgs "$@" && set -- "${_cmdArgs[@]}"

	local -a evalArgs=()
	local -a installables=()
	while test $# -gt 0; do
		case "$1" in
		-A | --attr) # takes one arg
			# legacy nix-build option; convert to flakeref
			shift
			installables+=(".#$1"); shift
			;;

		# All remaining options are `nix eval` args.

		# Options taking one arg.
		--apply|-write-to)
			evalArgs+=("$1"); shift
			evalArgs+=("$1"); shift
			;;
		# Options taking zero args.
		--debugger|--json|--raw|--read-only)
			evalArgs+=("$1"); shift
			;;
		# Assume all other options are installables.
		*)
			installables+=("$1"); shift
			;;
		esac

	done

	# If no installables specified then try identifying attrPath from
	# capacitated flake.
	if [ ${#installables[@]} -eq 0 ]; then
		local attrPath="$(selectAttrPath . eval packages)"
		installables=(".#$attrPath")
	fi

	if [ -n "$FLOX_ORIGINAL_NIX_GET_COMPLETIONS" ]; then
		export NIX_GET_COMPLETIONS="$(( FLOX_ORIGINAL_NIX_GET_COMPLETIONS + 1 ))"
	fi
	$invoke_nix "${_nixArgs[@]}" eval --impure "${evalArgs[@]}" "${installables[@]}" --override-input flox-floxpkgs/nixpkgs/nixpkgs flake:nixpkgs-$FLOX_STABILITY
}

#
# flakeTopLevel()
#
# Analogous to "git rev-parse --show-toplevel", this subroutine will
# identify the "toplevel" directory of the flake being evaluated, even
# if it's a subflake of a monorepo.
#
function flakeTopLevel() {
	trace "$@"
	local flakeRef=$1; shift
	local url
	if url=$($invoke_nix "${_nixArgs[@]}" flake metadata "$flakeRef" --json "$@" --override-input flox-floxpkgs/nixpkgs/nixpkgs flake:nixpkgs-$FLOX_STABILITY 2>/dev/null </dev/null | $_jq -r .resolvedUrl); then
		# strip git+file://
		url="${url/git+file:\/\//}"
		# strip path:
		url="${url/path:/}"
		# extract dir for subflakes if it exists
		dir="$(echo "$url" | $_sed -rn 's/.*\?dir=([^&]*).*/\1/p')"
		# strip ?*
		url="${url/\?*/}"
		if [ -n "$dir" ]; then
			echo "$url/$dir"
		else
			echo "$url"
		fi
	else
		exit 1
	fi
}

#
# floxProjectMetaDir()
#
# Returns the path of the ".flox" subdirectory at the "toplevel" of
# a given flake, which can be any of:
#
# 1. the root directory of a git clone (most common)
# 2. a "subflake" subdirectory of a git clone
# 3. an arbitrary directory not within a git clone
#
# In the first two cases we prompt to add the directory to .gitignore.
#
function flakeMetaDir {
	trace "$@"
	local topLevel=$1; shift
	local metaDir="$topLevel/.flox"
	[ -d $metaDir ] || $invoke_mkdir -p "$metaDir"

	local gitCloneToplevel
	if false && gitCloneToplevel="$($_git -C $topLevel rev-parse --show-toplevel 2>/dev/null)"; then
		local metaSubDir=${metaDir/$gitCloneToplevel\///}
		# TODO: re-enable following more extensive testing
		if [ $interactive -eq 1 ]; then
			if ! $_grep -q "^$metaSubDir$" "$gitCloneToplevel/.gitignore" && \
				$invoke_gum confirm "add $metaSubDir to toplevel .gitignore file?"; then
				echo "$metaSubDir" >> "$gitCloneToplevel/.gitignore"
				$invoke_git -C "$gitCloneToplevel" add .gitignore
				warn "updated $gitCloneToplevel/.gitignore"
			fi
		fi
	fi
	echo $metaDir
}

# flox develop, aka flox print-dev-env when run non-interactively
_development_commands+=("develop")
_usage["develop"]="launch development shell for current project"
_development_commands+=("print-dev-env")
_usage["print-dev-env"]="print shell code that can be sourced by bash to reproduce the development environment"
function floxDevelop() {
	trace "$@"
	parseNixArgs "$@" && set -- "${_cmdArgs[@]}"

	local -a developArgs=()
	local -a installables=()
	local -a remainingArgs=()
	while test $# -gt 0; do
		case "$1" in
		-A | --attr) # takes one arg
			# legacy nix-build option; convert to flakeref
			shift
			installables+=(".#$1"); shift
			;;

		# All remaining options are `nix build` args.

		# Options taking two args.
		--redirect|--arg|--argstr|--override-flake|--override-input)
			developArgs+=("$1"); shift
			developArgs+=("$1"); shift
			developArgs+=("$1"); shift
			;;
		# Options taking one arg.
		--keep|-k|--phase|--profile|--unset|-u|--eval-store|--include|-I|--inputs-from|--update-input|--expr|--file|-f)
			developArgs+=("$1"); shift
			developArgs+=("$1"); shift
			;;
		# Options that consume remaining arguments
		--command|-c)
			remainingArgs+=("$@")
			break
			;;
		# Options taking zero args.
		-*)
			developArgs+=("$1"); shift
			;;
		# Assume first unknown option is an installable and the rest are for commands.
		*)
			if [ ${#installables[@]} -eq 0 ]; then
				installables=("$1"); shift
			else
				remainingArgs+=("$1"); shift
			fi
			;;
		esac

	done

	# If no installables specified then try identifying attrPath from
	# capacitated flake.
	if [ ${#installables[@]} -eq 0 ]; then
		local attrPath="$(selectAttrPath . develop packages floxEnvs devShells)"
		installables=(".#$attrPath")
	fi

	# There can only be one installable with flox develop.
	local installable
	if [ ${#installables[@]} -gt 1 ]; then
		error "flox develop only accepts 1 installable" </dev/null
	else
		installable="${installables[0]}"
	fi

	# Start by parsing installable into its flakeref and attrpath parts.

	# If the user has provided the fully-qualified attrPath then remove
	# the "packages.$system." part as we'll add it back when constructing
	# the canonical flake URLs.
	local installableAttrPath=${installable//*#/}
	installableAttrPath="${installableAttrPath//packages.$FLOX_SYSTEM./}"

	# If the user didn't provide the url part of the flakeref then use ".".
	local installableFlakeRef=${installable//#*/}
	if [ "$installableFlakeRef" == "$installable" ]; then
		installableFlakeRef="."
	fi

	# Compute the canonical build and floxEnv flakerefs for the installable.
	local floxEnvFlakeURL="${installableFlakeRef}#.floxEnvs.$FLOX_SYSTEM.$installableAttrPath"
	local packageFlakeURL="${installableFlakeRef}#.packages.$FLOX_SYSTEM.$installableAttrPath"
	local devShellFlakeURL="${installableFlakeRef}#.devShells.$FLOX_SYSTEM.$installableAttrPath"

	# Compute the GCRoot path to be created/activated.
	local topLevel
	topLevel=$(flakeTopLevel "$installableFlakeRef" "${developArgs[@]}")
	[ -n "$topLevel" ] || \
		error "could not determine toplevel directory from '$installableFlakeRef' (syntax error?)" < /dev/null
	local metaDir
	metaDir=$(flakeMetaDir "$topLevel")
	local floxEnvGCRoot="$metaDir/envs/$FLOX_SYSTEM.$installableAttrPath"
	local floxNixStorePath
	floxNixStorePath="$($invoke_nix eval "$floxEnvFlakeURL".meta.position --impure --raw 2>/dev/null || true)"

	# Figure out whether we're operating in a floxified project.
	if [ -n "$floxNixStorePath" ]; then
		# The flox "happy path" is to stick to only the package derivations
		# and associated floxEnvs, so take this opportunity to rewrite the
		# installable to be the specific package derivation only and hide
		# anything else (devShells, etc.) that is apt to confuse and distract.
		installable="$packageFlakeURL"
	else
		# Let Nix decide whether to use a package or a devShell
		installable="$installableFlakeRef#$installableAttrPath"
	fi

	if [ -n "$FLOX_ORIGINAL_NIX_GET_COMPLETIONS" ]; then
		# Dispatch nix to perform the work of looking up matches for $installable.
		export NIX_GET_COMPLETIONS="$(( FLOX_ORIGINAL_NIX_GET_COMPLETIONS + 1 ))"
		verboseExec $_nix "${_nixArgs[@]}" develop "$installable" "${developArgs[@]}" \
			--override-input flox-floxpkgs/nixpkgs/nixpkgs flake:nixpkgs-$FLOX_STABILITY \
			"${remainingArgs[@]}"
	else

		local nixDevelopInvocation
		if [ $interactive -eq 1 ]; then
			nixDevelopInvocation="$_nix ${_nixArgs[*]} develop $installable ${developArgs[*]} \
						--override-input flox-floxpkgs/nixpkgs/nixpkgs flake:nixpkgs-$FLOX_STABILITY \
						${remainingArgs[*]}"
		else
			nixDevelopInvocation="$_nix ${_nixArgs[*]} print-dev-env $installable ${developArgs[*]} \
						--override-input flox-floxpkgs/nixpkgs/nixpkgs flake:nixpkgs-$FLOX_STABILITY \
						${remainingArgs[*]}"
		fi
		#
		# Next steps:
		# 1. build floxEnv for the installable (if there is one), creating a
		#    GCRoot for that package in the process
		# 2. activate the newly-rendered floxEnv by way of the GCRoot path
		# 3. finish by exec'ing "nix develop" or "nix print-dev-env" for the
		#    installable's package flakeref

		if [ -n "$floxNixStorePath" ]; then
			local floxNixDir
			floxNixDir="$topLevel/${floxNixStorePath#/*/*/*/}"
			floxNixDir="$($_dirname "$floxNixDir")"
			# Try to build the floxEnv if there is one.
			# The following build could fail; let it.
			floxBuild "${_nixArgs[@]}" --out-link "$floxEnvGCRoot" "$floxEnvFlakeURL" "${developArgs[@]}" || \
				error "failed to build floxEnv from $floxNixDir/flox.nix" < /dev/null

			# The build was successful so copy the newly rendered catalog and
			# manifest data into the installable directory.
			$_jq . --sort-keys "$floxEnvGCRoot/catalog.json" > "$floxNixDir/catalog.json"
			$_jq . --sort-keys "$floxEnvGCRoot/manifest.json" > "$floxNixDir/manifest.json"

			# Only attempt layering when associated package exists
			if ! $invoke_nix eval "$packageFlakeURL".name --impure --raw 2>/dev/null >/dev/null; then
					nixDevelopInvocation=""
			fi
			# That's all there is to it - just hand over control to flox activate
			# to take it from here.
			# flox develop
			if [ $interactive -eq 1 ]; then
				floxActivate "$floxEnvFlakeURL" "$FLOX_SYSTEM" -- $nixDevelopInvocation
			# print-dev-env
			else
				floxActivate "$floxEnvFlakeURL" "$FLOX_SYSTEM"
				verboseExec $nixDevelopInvocation
			fi
		else
			# Otherwise we just proceed with 'nix (develop|print-dev-env)'.
			verboseExec $nixDevelopInvocation
		fi
	fi
}

# flox run
_development_commands+=("run")
_usage["run"]="run app from current project"
function floxRun() {
	trace "$@"
	parseNixArgs "$@" && set -- "${_cmdArgs[@]}"

	local -a runArgs=()
	local -a installables=()
	local -a remainingArgs=()
	while test $# -gt 0; do
		case "$1" in
		-A | --attr) # takes one arg
			# legacy nix-run option; convert to flakeref
			shift
			installables+=(".#$1"); shift
			;;

		# All remaining options are `nix run` args.

		# Options taking two args.
		--arg|--argstr|--override-flake|--override-input)
			runArgs+=("$1"); shift
			runArgs+=("$1"); shift
			runArgs+=("$1"); shift
			;;
		# Options taking one arg.
		--eval-store|--include|-I|--inputs-from|--update-input|--expr|--file|-f)
			runArgs+=("$1"); shift
			runArgs+=("$1"); shift
			;;
		# Options that consume remaining arguments
		--)
			remainingArgs+=("$@")
			break
			;;
		# Options taking zero args.
		-*)
			runArgs+=("$1"); shift
			;;
		# nix will potentially still grab args after the installable, but we have no need to parse them
		# we aren't grabbing any flox specific args though, so flox run .#installable --arg-for-flox won't
		# work
		*)
			if [ ${#installables[@]} -eq 0 ]; then
				installables=("$1"); shift
			else
				remainingArgs+=("$1"); shift
			fi
			;;
		esac

	done

	# If no installables specified then try identifying attrPath from
	# capacitated flake.
	if [ ${#installables[@]} -eq 0 ]; then
		local attrPath="$(selectAttrPath . run packages)"
		installables=(".#$attrPath")
	fi

	if [ -n "$FLOX_ORIGINAL_NIX_GET_COMPLETIONS" ]; then
		export NIX_GET_COMPLETIONS="$(( FLOX_ORIGINAL_NIX_GET_COMPLETIONS + 1 ))"
	fi
	$invoke_nix "${_nixArgs[@]}" run --impure "${runArgs[@]}" "${installables[@]}" --override-input flox-floxpkgs/nixpkgs/nixpkgs flake:nixpkgs-$FLOX_STABILITY "${remainingArgs[@]}"
}

# flox shell
_development_commands+=("shell")
_usage["shell"]="run a shell in which the current project is available"
function floxShell() {
	trace "$@"
	parseNixArgs "$@" && set -- "${_cmdArgs[@]}"

	local -a shellArgs=()
	local -a installables=()
	local -a remainingArgs=()
	while test $# -gt 0; do
		case "$1" in
		-A | --attr) # takes one arg
			# legacy nix-run option; convert to flakeref
			shift
			installables+=(".#$1"); shift
			;;

		# All remaining options are `nix run` args.

		# Options taking two args.
		--arg|--argstr|--override-flake|--override-input)
			shellArgs+=("$1"); shift
			shellArgs+=("$1"); shift
			shellArgs+=("$1"); shift
			;;
		# Options taking one arg.
		--keep|-k|--unset|-u|--eval-store|--include|-I|--inputs-from|--update-input|--expr|--file|-f)
			shellArgs+=("$1"); shift
			shellArgs+=("$1"); shift
			;;
		# Options that consume remaining arguments
		--command|-c)
			remainingArgs+=("$@")
			break
			;;
		# Options taking zero args.
		-*)
			shellArgs+=("$1"); shift
			;;
		# Assume all other options are installables.
		*)
			installables+=("$1"); shift
			;;
		esac

	done

	# If no installables specified then try identifying attrPath from
	# capacitated flake.
	if [ ${#installables[@]} -eq 0 ]; then
		local attrPath="$(selectAttrPath . shell packages)"
		installables=(".#$attrPath")
	fi

	if [ -n "$FLOX_ORIGINAL_NIX_GET_COMPLETIONS" ]; then
		export NIX_GET_COMPLETIONS="$(( FLOX_ORIGINAL_NIX_GET_COMPLETIONS + 1 ))"
	fi
	$invoke_nix "${_nixArgs[@]}" shell --impure "${shellArgs[@]}" "${installables[@]}" --override-input flox-floxpkgs/nixpkgs/nixpkgs flake:nixpkgs-$FLOX_STABILITY "${remainingArgs[@]}"
}

#
# selectDefaultEnvironment($defaultEnvironment)
#
# Looks to see if current directory is within a project with a
# capacitated flake, and if so then prompts for and returns the
# environment implied by the selected project.
#
function selectDefaultEnvironment() {
	trace "$@"
	local subcommand="$1"; shift
	local defaultEnv="$1"; shift

	case "$subcommand" in
	activate|edit|install|list|remove|upgrade) # support project environments
		: ;; # pass
	*) # all other commands do not support project environments
		echo "$defaultEnv"
		return 0
		;;
	esac

	local topLevel
	topLevel=$(flakeTopLevel ".") || :
	[ -n "$topLevel" ] || topLevel="."
	# This could fail noisily, so quietly try a lookup before calling
	# selectAttrPath() which needs to prompt to stderr.
	local -a attrPaths=($(lookupAttrPaths $topLevel floxEnvs 2>/dev/null))
	if [ ${#attrPaths[@]} -gt 0 ]; then
		local attrPath="$(selectAttrPath $topLevel $subcommand floxEnvs)"
		if [ -n "$attrPath" ]; then
			echo "$topLevel#$attrPath"
		else
			echo "$defaultEnv"
		fi
	else
		echo "$defaultEnv"
	fi
}

# vim:ts=4:noet:syntax=bash
