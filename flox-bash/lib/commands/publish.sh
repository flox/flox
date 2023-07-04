## Development commands

# Splitting out "flox publish" into its own module because it is
# quite a bit more complex than other commands and deserves to
# be split out into a collection of related functions.

#
# doEducatePublish()
#
# A very important function, gives users an overview of 'flox publish'
# and sets a flag to not present the same information more than once.
declare -i educatePublishCalled=0
function doEducatePublish() {
	educatePublishCalled=1
	[ $educatePublish -eq 0 ] || return 0
	$_cat <<EOF 1>&2

As this seems to be your first time publishing a package here's a
brief overview of the process.

Publishing a package requires the following:

  * the build repository from which to "flox build"
  * a package to be published within that repository
  * a channel repository for storing built package metadata
  * [optional] a binary cache location for storing copies
    of already-built packages
  * [optional] a binary cache location from which to
    download already-built packages for faster installation

Once it has been published to a channel repository, you can
search for and use your package with the following:

  * subscribe to the channel: flox subscribe <channel> <URL>
  * search for a package: flox search -c <channel> <package>
  * install a package: flox install <channel>.<package>

See the flox(1) man page for more information.

EOF
	educatePublish=1
	floxUserMetaRegistry setNumber educatePublish 1
}

#
# Create project-specific flox registry file.
# XXX TODO move to development command bootstrap logic?
#
declare gitCloneRegistry
#shellcheck disable=SC2120
function initProjectRegistry() {
	trace "$@"
	local gitCloneToplevel floxProjectMetaDir;
	gitCloneToplevel="$($_git rev-parse --show-toplevel || :)"
	floxProjectMetaDir=".flox"
	if [ -n "$gitCloneToplevel" ]; then
		local gitCloneFloxDir="$gitCloneToplevel/$floxProjectMetaDir"
		[ -d $gitCloneFloxDir ] || $invoke_mkdir -p "$gitCloneFloxDir"
		gitCloneRegistry="$gitCloneFloxDir/metadata.json"
		if [ $interactive -eq 1 ]; then
			[ -f $gitCloneRegistry ] || info "creating $gitCloneRegistry"
			if ! $_grep -q "^/$floxProjectMetaDir$" "$gitCloneToplevel/.gitignore" && \
				$invoke_gum confirm "add /$floxProjectMetaDir to toplevel .gitignore file?"; then
				echo "/$floxProjectMetaDir" >> "$gitCloneToplevel/.gitignore"
				$invoke_git -C "$gitCloneToplevel" add .gitignore
				warn "clone modified - please commit and re-invoke"
				exit 1
			fi
		fi
	fi
}

_development_commands+=("publish")
_usage["publish"]="build and publish project to flox channel"
_usage_options["publish"]="[--build-repo <URL>] [--channel-repo <URL>] \\
                 [--upload-to <URL>] [--download-from <URL>] \\
                 [--render-path <dir>] [--key-file <file>] \\
                 [(-A|--attr) <package>] [--publish-system <system>]"
function floxPublish() {
	trace "$@"
	parseNixArgs "$@" && set -- "${_cmdArgs[@]}"

	# Publish takes the same args as build, plus a few more.
	# Split out the publish args from the build args.
	local -a buildArgs=()
	local -a installables
	local packageAttrPath
	local packageFlakeRef
	local buildFlakeURL
	local canonicalFlakeRef
	local canonicalFlakeURL
	local buildRepository
	local channelRepository
	local uploadTo
	local downloadFrom
	local renderPath="catalog"
	local tmpdir
	tmpdir=$(mkTempDir)
	local gitClone # separate from tmpdir out of abundance of caution
	local keyFile
	local publishSystem=$FLOX_SYSTEM
	while test $# -gt 0; do
		case "$1" in
		# Required
		--build-repo | -b | --upstream-url) # XXX TODO: remove mention of upstream
			[ "$1" != "--upstream-url" ] || \
				warn "Warning: '$1' is deprecated - please use '--build-repo' instead"
			shift
			buildRepository="$1"; shift
			;;
		--channel-repo | -c | --publish-to | -p) # XXX TODO: remove mention of publish
			[ "$1" != "--publish-to" -a "$1" != "-p" ] || \
				warn "Warning: '$1' is deprecated - please use '--channel-repo' instead"
			shift
			channelRepository="$1"; shift
			;;
		# Optional
		--upload-to | --copy-to) # takes one arg
			[ "$1" != "--copy-to" ] || \
				warn "Warning: '$1' is deprecated - please use '--upload-to' instead"
			shift
			uploadTo="$1"; shift
			;;
		--download-from | --copy-from) # takes one arg
			[ "$1" != "--copy-from" ] || \
				warn "Warning: '$1' is deprecated - please use '--download-from' instead"
			shift
			downloadFrom="$1"; shift
			;;
		# Expert
		--render-path | -r) # takes one arg
			shift
			renderPath="$1"; shift
			;;
		--key-file | -k) # takes one arg
			shift
			keyFile="$1"; shift
			;;
		--publish-system) # takes one arg
			shift
			publishSystem="$1"; shift
			;;
		# Select package (installable)
		-A | --attr) # takes one arg
			shift
			installables+=(".#$1"); shift
			;;

		# All remaining options are `nix build` args.

		# Options taking two args.
		--out-link|-o|--profile|--override-flake|--override-input)
			buildArgs+=("$1"); shift
			buildArgs+=("$1"); shift
			buildArgs+=("$1"); shift
			;;
		# Options taking one arg.
		--eval-store|--include|-I|--inputs-from|--update-input|--expr|--file|-f)
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

	# Publishing a package requires answers to the following:
	#
	# 1) the "source" repository from which to "flox build" (the "flakeRef")
	# 2) a package to be published within that repository (the "attrPath")
	# 3) a "channel" repository for storing built package metadata
	# 4) (optional) a list of "binary cache" URLs for uploading signed
	#    copies of already-built packages
	# 5) (optional) a list of "binary cache" URLs from which to download
	#    already-built packages
	#
	# Walk the user through the process of collecting each of
	# these in turn.

	# If no installables specified then try identifying attrPath from
	# capacitated flake in current directory.
	if [ ${#installables[@]} -eq 0 ]; then
		packageAttrPath="$(selectAttrPath . publish packages)"
		packageFlakeRef="."
	elif [ ${#installables[@]} -eq 1 ]; then
		case ${installables[0]} in
		*"#"*)
			# extract {packageAttrPath,packageFlakeRef} from provided flakeURL.
			# Example: git+ssh://git@github.com/flox/floxpkgs-internal?ref=master&rev=ca38729e6ab6066331b30c874053f12828c4a24f
			packageAttrPath=${installables[0]//*#/}
			packageFlakeRef=${installables[0]//#*/}
			;;
		*)
			usage | error "invalid package reference: ${installables[0]}"
			;;
		esac
	else
		usage | error "multiple arguments provided to 'flox publish' command"
	fi

	# If the user has provided the fully-qualified attrPath then remove
	# the "packages.$publishSystem." part as we'll add it back for
	# those places below where we need it.
	packageAttrPath="${packageAttrPath//packages.$publishSystem./}"

	# First get our bearings with some data regarding the local git clone,
	# if in fact we are in a local git clone.
	local upstreamFullName upstreamRemote upstreamBranch
	local cloneRemote cloneBranch cloneRev
	if [ "$packageFlakeRef" = "." ] && $_git rev-parse --is-inside-work-tree > /dev/null; then
		# Start with a quick refresh, then make note of all local state.
		$_git fetch -q
		upstreamFullName="$($_git rev-parse --abbrev-ref --symbolic-full-name "@{u}")"
		upstreamRemote="${upstreamFullName//\/*/}"
		upstreamBranch="${upstreamFullName//${upstreamRemote}\//}"
		cloneRemote="$($_git remote get-url ${upstreamRemote:-origin})"
		cloneBranch="$($_git rev-parse --abbrev-ref --symbolic-full-name @)"
		cloneRev="$($_git rev-parse @)"
		# Create the project registry before proceeding.
		initProjectRegistry
	fi

	# The --build-repo argument specifies the repository of the flake used
	# to build the package. When invoked from a git clone without specifying
	# a full flake URL this defaults to its current remote URL.
	if [ -z "$buildRepository" ]; then
		doEducatePublish
		# Load previous answer (if applicable).
		if ! buildRepository=$(registry "$gitCloneRegistry" 1 get buildRepository); then
			# Derive the default flakeRef from the current git clone.
			if [ "$packageFlakeRef" = "." ]; then
				buildRepository="$cloneRemote"
			else
				buildRepository="$packageFlakeRef"
			fi
		fi
		while true; do
			if checkGitRepoExists "$buildRepository"; then
				[ -z "$buildRepository" ] || break
			fi
			warn "repository '$buildRepository' does not exist"
			warn "please enter a valid URL from which to 'flox build' a package"
			buildRepository=$(promptInput \
				"Enter git URL (required)" \
				"build repository:" \
				"$buildRepository")
		done
	fi
	warn "build repository: $buildRepository"

	# Canonicalize the full flake URL.
	case "$buildRepository" in
	*'?'*rev=*)
		# The buildRepository can be specified of the form $baseURL?$options
		# in which case we don't mess with it.
		canonicalFlakeRef="${buildRepository}"
		;;
	*)
		local buildRepositoryBase headref
		if [[ "$buildRepository" =~ '?'*ref= ]]; then
		    buildRepositoryBase="${buildRepository/\?*/}"
			headref="${buildRepository/*\?ref=/}"
			headref="${headref/&*/}"
		else
		    buildRepositoryBase="$buildRepository"
			headref="HEAD"
		fi

		# Figure out the HEAD version to derive canonical flake URL.
		local upstreamRev
		upstreamRev=$(githubHelperGit ls-remote "$buildRepositoryBase" "$headref")
		# Keep only first 40 characters to remove the extra spaces and "HEAD" label.
		upstreamRev=${upstreamRev:0:40}
		# If we did derive the buildRepository from a local git clone, confirm
		# that it is not out of sync with upstream.
		if [ "$buildRepositoryBase" = "$cloneRemote" ]; then
			if ! $_git diff --exit-code --quiet; then
				warn "Warning: uncommitted changes detected"
				error "commit all changes before publishing" < /dev/null
			fi
			if ! $_git diff --cached --exit-code --quiet; then
				warn "Warning: staged commits not present in upstream rev ${upstreamRev:0:7}"
				if [[ "${interactive:-0}" -eq 1 ]]; then
					if ! $invoke_gum confirm "proceed to publish revision ${upstreamRev:0:7}?"; then
						warn "aborting ..."
						exit 1
					fi
				else
					warn "aborting ..."
					exit 1
				fi
			fi
			if [ "$cloneRev" != "$upstreamRev" ]; then
				warn "Warning: local clone (${cloneRev:0:7}) out of sync with upstream (${upstreamRev:0:7})"
				if [[ "${interactive:-0}" -eq 1 ]]; then
					if $invoke_gum confirm "push revision ${cloneRev:0:7} upstream and publish?"; then
						warn "+ $_git push"
						$_git push
						upstreamRev="$cloneRev"
					else
						warn "aborting ..."
						exit 1
					fi
				else
					warn "aborting ..."
					exit 1
				fi
			fi
		fi
		canonicalFlakeRef="${buildRepositoryBase}?rev=${upstreamRev}"
		;;
	esac

	# Nix m'annoye.
	case "$canonicalFlakeRef" in
		git@* )
			canonicalFlakeRef="git+ssh://${canonicalFlakeRef/[:]//}"
			;;
		ssh://* | http://* | https://* )
			canonicalFlakeRef="git+$canonicalFlakeRef"
			;;
	esac

	# The -A argument specifies the package to be built within the
	# $buildRepository if it's not provided in an explicit flakeURL.
	if [ -z "$packageAttrPath" ]; then
		doEducatePublish
		packageAttrPath="$(selectAttrPath "$canonicalFlakeRef" publish packages)"
	fi

	# Stash the canonical and build flake URLs before altering $packageAttrPath.
	canonicalFlakeURL="${canonicalFlakeRef}#${packageAttrPath}"
	buildFlakeURL="${buildRepository}#${packageAttrPath}"

	# The packageAttrPath as constructed by Hydra will be of the form
	# <flakeRef>#hydraJobsStable.<pname>.<system>. Take this opportunity
	# to extract the pname. XXX Still needed?
	case "$packageAttrPath" in
	hydraJobsStable.*.$publishSystem)
		FLOX_STABILITY=stable
		packageAttrPath="${packageAttrPath//hydraJobsStable./}"
		packageAttrPath="${packageAttrPath//.$publishSystem/}"
		;;
	hydraJobsStaging.*.$publishSystem)
		FLOX_STABILITY=staging
		packageAttrPath="${packageAttrPath//hydraJobsStaging./}"
		packageAttrPath="${packageAttrPath//.$publishSystem/}"
		;;
	hydraJobsUnstable.*.$publishSystem)
		FLOX_STABILITY=unstable
		packageAttrPath="${packageAttrPath//hydraJobsUnstable./}"
		packageAttrPath="${packageAttrPath//.$publishSystem/}"
		;;
	esac
	warn "package name: $packageAttrPath"

	# The --channel-repo argument specifies the repository for storing
	# built package metadata. When invoked from a git clone this defaults
	# to a "floxpkgs" repository in the same organization as its origin.
	if [ -z "$channelRepository" ]; then
		doEducatePublish
		# Load previous answer (if applicable).
		if ! channelRepository=$(registry "$gitCloneRegistry" 1 get channelRepository); then
			case "$origin" in
				*/* )
					channelRepository=$($_dirname "$origin")/floxpkgs
					;;
			esac
		fi
		while true; do
			if ensureGHRepoExists "$channelRepository" private "https://github.com/flox/floxpkgs-template.git"; then
				[ -z "$channelRepository" ] || break
			fi
			warn "please enter a valid URL with which to 'flox subscribe'"
			channelRepository=$(promptInput \
				"Enter git URL (required)" \
				"channel repository:" \
				"$channelRepository")
		done
	fi
	warn "channel repository: $channelRepository"

	# Prompt for location(s) TO and FROM which we can (optionally) copy the
	# built package store path(s). By default these will refer to the same
	# URL, but can be overridden with --download-from.
	if [[ -z "${uploadTo+1}" ]]; then
		doEducatePublish
		# Load previous answer (if applicable).
		uploadTo="$(registry "$gitCloneRegistry" 1 get uploadTo || :)"
		if [[ -z "$uploadTo" ]] && [[ "${interactive:-0}" -eq 1 ]]; then
			# XXX TODO: find a way to remember previous binary cache locations
			uploadTo="$(promptInput                                    \
				"Enter binary cache URL (leave blank to skip upload)"  \
				"binary cache for upload:"                             \
				"$uploadTo")"
		fi
	fi
	# Set empty fallback
	: "${uploadTo:=}"
	[[ -z "${uploadTo:-}" ]] || warn "upload to: $uploadTo"

	if [[ -z "${downloadFrom+1}" ]]; then
		# Load previous answer (if applicable).
		downloadFrom="$(registry "$gitCloneRegistry" 1 get downloadFrom || :)"
		if [[ -z "$downloadFrom" ]] && [[ "${interactive:-0}" -eq 1 ]]; then
			# Note - the following line is not a mistake; if $downloadFrom is not
			# defined then we should use $uploadTo as the default suggested value.
			downloadFrom="$(promptInput              \
				"Enter binary cache URL (optional)"  \
				"binary cache for download:"         \
				"$uploadTo")"
		fi
	fi
	# Set empty fallback
	: "${downloadFrom:=}"
	[[ -z "$downloadFrom" ]] || warn "download from: $downloadFrom"

	# Construct string encapsulating entire command invocation.
	local entirePublishCommand
	entirePublishCommand="$(printf \
		"flox publish -A %s --build-repo %s --channel-repo %s" \
		"$packageAttrPath" "$buildRepository" "$channelRepository")"
	[ -z "$uploadTo" ] || entirePublishCommand="$(printf "%s --upload-to %s" "$entirePublishCommand" "$uploadTo")"
	[ -z "$downloadFrom" ] || entirePublishCommand="$(printf "%s --download-from %s" "$entirePublishCommand" "$downloadFrom")"

	# Only hint and save responses in interactive mode.
	if [[ "${interactive:-0}" -eq 1 ]]; then
		# Input parsing over, print informational hint in the event that we
		# had to ask any questions.
		if [[ "${educatePublishCalled:-0}" -eq 1 ]]; then
			echo '{{ Color "'$LIGHTPEACH256'" "'$DARKBLUE256'" "$ '"$entirePublishCommand"'" }}' | \
				$_gum format -t template 1>&2
		fi

		# Save answers to the project registry so they can serve as
		# defaults for next time.
		if [[ -n "${gitCloneRegistry:-}" ]]; then
			registry "$gitCloneRegistry" 1 set buildRepository "$buildRepository"
			registry "$gitCloneRegistry" 1 set packageAttrPath "$packageAttrPath"
			registry "$gitCloneRegistry" 1 set channelRepository "$channelRepository"
			registry "$gitCloneRegistry" 1 set uploadTo "$uploadTo"
			registry "$gitCloneRegistry" 1 set downloadFrom "$downloadFrom"
		fi
	else
		echo '{{ Color "'$LIGHTPEACH256'" "'$DARKBLUE256'" "'"$entirePublishCommand"'" }}' | \
			$_gum format -t template 1>&2
	fi

	# Start by making sure we can clone the channel repository to
	# which we want to publish.
	if [[ "$channelRepository" = "-" ]]; then
		gitClone="-"
	elif [[ -d "$channelRepository" ]]; then
		gitClone="$channelRepository"
	else
		gitClone="$tmpdir"
		warn "Cloning $channelRepository ..."
		$invoke_gh repo clone "$channelRepository" "$gitClone"
	fi

	# Then build package.
	warn "Building $packageAttrPath ..."
	declare -a outpaths
	mapfile -t outpaths < <(
		floxBuild "${_nixArgs[@]}" --no-link --print-out-paths  \
		          "$canonicalFlakeURL^"'*' "${buildArgs[@]}"
    ) || error "could not build $canonicalFlakeURL" < /dev/null
	if [[ "${#outpaths[@]}" -le 0 ]]; then
		error "no outputs created from build of $canonicalFlakeURL" < /dev/null
	fi

	# TODO Make content addressable (uncomment "XXX" lines below).
	if false; then # XXX
	declare -a ca_out
	mapfile -t ca_out < <(
		$invoke_nix "${_nixArgs[@]}" store make-content-addressed             \
		            "${outpaths[@]}" --json|$_jq '.rewrites[]'||echo 'ERROR'
	)
	if [[ "${#ca_out[@]}" -gt 0 ]] && [[ "${#ca_out[0]}" != 'ERROR' ]]; then
		# Replace package outpaths with CA versions.
		warn "Replacing with content-addressable package: $ca_out"
		outpaths=( "${ca_out[@]}" )
	fi
	fi # /XXX

	# Sign the package outpaths (optional). Sign by default?
	if [[ -z "${keyFile:-}" ]] && [[ -f "$FLOX_CONFIG_HOME/secret-key" ]]; then
		keyFile="$FLOX_CONFIG_HOME/secret-key"
	fi
	if [[ -n "${keyFile:-}" ]]; then
		if [[ -f "$keyFile" ]]; then
			$invoke_nix "${_nixArgs[@]}" store sign -r --key-file "$keyFile"  \
				        "${outpaths[@]}"
		else
			error "could not read $keyFile: $!" < /dev/null
		fi
	fi

	### Next section cribbed from: github:flox/catalog-ingest#analyze

	# Analyze package.
	# TODO: bundle lib/analysis.nix with flox CLI to avoid dependency on remote flake
	local analyzer="path:$_lib/catalog-ingest"
	# Nix eval command is noisy so filter out the expected output.
	local tmpstderr
	tmpstderr="$(mkTempFile)"
	evalAndBuild=$($invoke_nix "${_nixArgs[@]}" eval --json \
		--override-input target "$canonicalFlakeRef" \
		--override-input target/flox-floxpkgs/nixpkgs/nixpkgs flake:nixpkgs-$FLOX_STABILITY \
		"$analyzer#.analysis.eval.packages.$publishSystem.$packageAttrPath" 2>$tmpstderr) || {
		$_grep --no-filename -v \
		  -e "^evaluating 'catalog\." \
		  -e "not writing modified lock file of flake" \
		  -e " Added input " \
		  -e " follows " \
		  -e "\([0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]\)" \
		  $tmpstderr 1>&2 || true
		error "eval of $analyzer#analysis.eval.packages.$publishSystem.$packageAttrPath failed - see above" < /dev/null
	}

	# Gather buildRepository package outpath metadata.
	local buildMetadata
	buildMetadata=$($invoke_nix "${_nixArgs[@]}" flake metadata "$canonicalFlakeRef" --no-write-lock-file --json)

	# Since jq variables don't need to be quoted:
	#shellcheck disable=SC2016
	evalAndBuildAndSource=$($_jq -n \
		--argjson evalAndBuild "$evalAndBuild" \
		--argjson buildMetadata "$buildMetadata" \
		--arg stability "$FLOX_STABILITY" '
		$evalAndBuild * {
			"element": {"url": "\($buildMetadata.resolvedUrl)"},
			"source": {
				locked: $buildMetadata.locked,
				original: $buildMetadata.original,
				remote: $buildMetadata.original
			},
			"eval": {
				"stability": $stability
			}
		}
	')

	# Copy to binary cache (optional).
	if [[ -n "$uploadTo" ]]; then
		local builtfilter="flake:flox#builtfilter"
		$invoke_nix "${_nixArgs[@]}" copy --to "$uploadTo" "${outpaths[@]}"
		# Enhance eval data with remote binary substituter.
		evalAndBuildAndSource=$(echo "$evalAndBuildAndSource" | \
			$invoke_nix "${_nixArgs[@]}" run "$builtfilter" -- --substituter "$downloadFrom")
	fi

	### Next section cribbed from: github:flox/catalog-ingest#publish
	warn "publishing render to $renderPath ..."

	#shellcheck disable=SC2016
	elementPath=$($_jq -n --sort-keys \
		--argjson evalAndBuildAndSource "$evalAndBuildAndSource" \
		--arg rootPath "$gitClone/$renderPath" '
		{
			"analysis": ($evalAndBuildAndSource),
			"attrPath": (
				"\($rootPath)/" + (
					$evalAndBuildAndSource.eval |
					[.system, .stability, .namespace, .version] |
					flatten |
					join("/")
				) + ".json"
			)
		}
	')

	if [ "$channelRepository" != "-" ]; then
		local epAttrPath
		epAttrPath="$($_jq -r .attrPath <<< "$elementPath")"
		$_mkdir -p "$($_dirname "$epAttrPath")"
		echo "$elementPath" | $_jq -r '.analysis' > "$( echo "$elementPath" | $_jq -r '.attrPath' )"
		warn "flox publish completed"
		$_git -C "$gitClone" add $renderPath
		if [ ! -d "$channelRepository" ]; then
			# TODO: improve contents of git log message
			epAttrPath="${epAttrPath//$gitClone\/$renderPath\//}"
			packageAttrPath="${packageAttrPath//packages.$publishSystem./}"
			printf "published %s\n\nURL: %s\nAttrPath: %s\nSystem: %s\nStability: %s\nUser: %s\nCommand: %s\n" \
				"$epAttrPath" "$buildFlakeURL" "$packageAttrPath" "$publishSystem" \
				"$FLOX_STABILITY" "$USER" "$invocation_string" | \
			if $_git -C "$gitClone" commit -F -; then
				# Attempt 3 times (arbitrary) to push commit upstream.
				local -i pushAttempt;
				pushAttempt=0
				while true; do
					[ "$pushAttempt" -lt 3 ] ||
						error "could not push to $channelRepository after $pushAttempt attempts" </dev/null
					githubHelperGit -C "$gitClone" pull --rebase
					if githubHelperGit -C "$gitClone" push; then
						# Job done.
						break
					else
						(( ++pushAttempt ))
						# Give it an increasing delay before attempting to push again.
						sleep $pushAttempt
					fi
				done
			fi
		fi
	else
		#shellcheck disable=2016
		$_jq -n -r --argjson ep "$elementPath" '$ep.analysis'
	fi
}
# vim:ts=4:noet:syntax=bash
