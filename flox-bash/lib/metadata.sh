#
# Subroutines for management of "floxmeta" environment metadata repo.
#
# This module provides functions to manage the user's environment metadata
# repository in conjunction with the generational links pointing to the flox
# environment packages in the store.
#
# The profile metadata repository contains copies of all source files required
# to create each generation in a subdirectory corresponding with the generation
# number. This includes a flake.{nix,lock} pair which enables the directory to
# be built as a standalone package if desired.
#
# There is one metadata repository per user and each profile is represented
# as a separate branch. See https://github.com/flox/flox/issues/14.
#

# Example hierarchy (temporary during refactoring):
# .
# ├── limeytexan (x86_64-linux.default branch)
# │   ├── 1
# │   │   ├── manifest.toml
# │   │   └── manifest.json
# │   └── metadata.json
# ├── limeytexan (x86_64-linux.toolbox branch)
# │   ├── 1
# │   │   ├── manifest.toml
# │   │   └── manifest.json
# │   ├── 2
# │   │   ├── manifest.toml
# │   │   └── manifest.json
# │   └── metadata.json
# └── tomberek (aarch64-darwin.default branch)
#     ├── 1
#     │   ├── manifest.toml
#     │   └── manifest.json
#     ├── 2
#     │   ├── manifest.toml
#     │   └── manifest.json
#     ├── 3
#     │   ├── manifest.toml
#     │   └── manifest.json
#     └── metadata.json

# Example hierarchy (unification):
# .
# ├── limeytexan (x86_64-linux.default branch)
# │   ├── 1
# │   │   ├── flake.lock
# │   │   ├── flake.nix
# │   │   └── pkgs
# │   │       └── default
# │   │           ├── catalog.json
# │   │           └── flox.nix
# │   └── metadata.json
# ├── limeytexan (x86_64-linux.toolbox branch)
# │   ├── 1
# │   │   ├── flake.lock
# │   │   ├── flake.nix
# │   │   └── pkgs
# │   │       └── default
# │   │           ├── catalog.json
# │   │           └── flox.nix
# │   ├── 2
# │   │   ├── flake.lock
# │   │   ├── flake.nix
# │   │   └── pkgs
# │   │       └── default
# │   │           ├── catalog.json
# │   │           └── flox.nix
# │   └── metadata.json
# └── tomberek (aarch64-darwin.default branch)
#     ├── 1
#     │   ├── flake.lock
#     │   ├── flake.nix
#     │   └── pkgs
#     │       └── default
#     │           ├── catalog.json
#     │           └── flox.nix
#     ├── 2
#     │   ├── flake.lock
#     │   ├── flake.nix
#     │   └── pkgs
#     │       └── default
#     │           ├── catalog.json
#     │           └── flox.nix
#     ├── 3
#     │   ├── flake.lock
#     │   ├── flake.nix
#     │   └── pkgs
#     │       └── default
#     │           ├── catalog.json
#     │           └── flox.nix
#     └── metadata.json

#
# "Public" functions exposed by this module:
#
# * syncEnvironment(): reconciles/updates profile data from metadata repository
# * pullMetadata(): pulls metadata updates from upstream to local cache
# * pushMetadata(): pushes metadata updates from local cache to upstream
# * metaGit():      provides access to git commands for metadata repo
# * metaGitShow():  used to print file contents without checking out branch
#
# Many git conventions employed here are borrowed from Nix's own
# src/libfetchers/git.cc file.
#

snipline="------------------------ >8 ------------------------"
declare protoManifestToml
protoManifestToml=$($_cat <<EOF
# This is a prototype profile declarative manifest in TOML format,
# supporting comments and the ability to invoke "shellHook" commands
# upon profile activation. See the flox(1) man page for more details.

# [environment]
#   LANG = "en_US.UTF-8"
#   LC_ALL = "\$LANG"
#
# [aliases]
#   gg = "git grep"
#
# [hooks]
#   sayhi = """
#     echo "Supercharged by flox!" 1>&2
#   """
#
# Edit below the "--- >8 ---" delimiter to define the list of packages to
# be installed, but note that comments and the ordering of packages will
# *not* be preserved with updates.

# $snipline
EOF
)

#
# gitInitFloxmeta($repoDir,$defaultBranch)
#
declare defaultBranch="floxmain"
function gitInitFloxmeta() {
	trace "$@"
	local repoDir="$1"; shift
	# Set initial branch with `-c init.defaultBranch=` instead of
	# `--initial-branch=` to stay compatible with old version of
	# git, which will ignore unrecognized `-c` options.
	$invoke_git -c init.defaultBranch="${defaultBranch}" init --quiet "$repoDir"
	$invoke_git -C "$repoDir" config pull.rebase true
	$invoke_git -C "$repoDir" config receive.denyCurrentBranch updateInstead
	# A commit is needed in order to make the branch visible.
	$invoke_git -C "$repoDir" commit --quiet --allow-empty \
		-m "$USER created repository"
}

# XXX TEMPORARY function to convert old-style "1.json" -> "1/manifest.json"
#     **Delete after 20221215**
function temporaryAssert007Schema {
	trace "$@"
	local repoDir="$1"; shift

	# Use the presence of manifest.toml in the top directory as
	# an indication that the repository has NOT been converted.
	[ -e "$repoDir/manifest.toml" ] || return 0

	# Prompt user to confirm they want to change the format.
	warn "floxmeta repository ($repoDir) using deprecated (<=0.0.6) format."
	$invoke_gum confirm "Convert to latest (>=0.0.7) format?"

	# Rename/move each file.
	for file in $($_git -C "$repoDir" ls-files); do
		case "$file" in
		[0-9]*.json)
			local gen
			gen=$($_basename "$file" .json)
			$invoke_mkdir -p "$repoDir/${gen}"
			$invoke_git -C "$repoDir" mv "$file" "${gen}/manifest.json"
			# Constructing the manifest.toml is not as straightforward.
			# The pre-0.0.7 format didn't include a generation-specific
			# manifest.toml, but rather forced you to go back to a previous
			# git commit to find the corresponding version. Worse than that,
			# when doing rollbacks and other generation flips the top half
			# of the manifest.toml didn't change, which was arguably wrong
			# (although appreciated as a feature by some).
			#
			# To create the old generation-specific manifest start by
			# including everything up to the snipline.
			$invoke_git -C "$repoDir" show "HEAD:manifest.toml" | \
				$_awk "{if (/$snipline/) {exit} else {print}}" > "$repoDir/$gen/manifest.toml"
			# Then use the current generation's manifest.json to create
			# the rest.
			echo "# $snipline" >> "$repoDir/$gen/manifest.toml"
			manifest "$repoDir/$gen/manifest.json" listEnvironmentTOML >> "$repoDir/$gen/manifest.toml"
			$invoke_git -C "$repoDir" add "$gen/manifest.toml"
			;;
		manifest.json)
			$invoke_git -C "$repoDir" rm "$file" ;;
		manifest.toml)
			$invoke_git -C "$repoDir" rm "$file" ;;
		metadata.json)
			: leave intact ;;
		*)
			error "unknown file \"$file\" in $repoDir repository" < /dev/null
			;;
		esac
	done

	# Commit, reading commit message from STDIN.
	$invoke_git -C "$repoDir" commit \
		--quiet -m "$USER converted to 0.0.7 floxmeta schema"
	$invoke_git -C $repoDir push --quiet

	warn "Conversion complete. Please re-run command."
	exit 0
}
# /XXX

# XXX TEMPORARY function to convert nix-profile-style "1/manifest.toml" -> "1/pkgs/default/flox.nix"
#     **Delete after 20230222**
function temporaryAssert008Schema {
	trace "$@"
	local environment="$1"; shift
	local repoDir="$1"; shift
	# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
	eval $(decodeEnvironment "$environment")
	local currentGen
	currentGen=$($_readlink $workDir/current || :)
	local nextGen
	nextGen=$($_readlink $workDir/next)
	local currentGenDir="$repoDir/$currentGen"
	local nextGenDir="$repoDir/$nextGen"

	# Use the presence of manifest.toml in the current generation as
	# an indication that the repository has NOT been converted.
	[ -e "$currentGenDir/manifest.toml" ] || return 0

	# Prompt user to confirm they want to change the format.
	warn "floxmeta repository ($currentGenDir) using deprecated (<=0.0.7) format."
	$invoke_gum confirm "Convert to latest (>=0.0.8) format?"

	# Copy the template flox environment into the next generation.
	# Files in the Nix store are read-only.
	$_cp --no-preserve=mode -rT $_lib/templateFloxEnv $nextGenDir
	# otherwise Nix build won't be able to find any of the files
	$_git -C $workDir add $nextGen

	# Use nix-editor to transfer packages from the current manifest.json file.
	local tmpScript
	tmpScript=$(mkTempFile)
	manifest $currentGenDir/manifest.json convert007to008 $_nix_editor $nextGenDir/pkgs/default/flox.nix > $tmpScript

	# Similarly use nix-editor to transfer aliases and env vars from manifest.toml.
	# jq outputs something like 'value'. Arguments to nix-editor have to be double quoted, so wrap with
	# '"', resulting in '"''value''"'
	$invoke_dasel -w json -f $currentGenDir/manifest.toml | \
		$invoke_jq -r --arg dq "'\"'" --arg nixEditor $_nix_editor --arg file $nextGenDir/pkgs/default/flox.nix \
			'(.aliases//{}) | to_entries | map(($dq+(.value|@sh)+$dq) as $quotedValue | "\($nixEditor) -i \($file) shell.aliases.\(.key) -v \($quotedValue)")[]' >> $tmpScript
	$invoke_dasel -w json -f $currentGenDir/manifest.toml | \
		$invoke_jq -r --arg dq "'\"'" --arg nixEditor $_nix_editor --arg file $nextGenDir/pkgs/default/flox.nix \
			'(.environment//{}) | to_entries | map(($dq+(.value|@sh)+$dq) as $quotedValue | "\($nixEditor) -i \($file) environmentVariables.\(.key) -v \($quotedValue)")[]' >> $tmpScript

	if [ $verbose -gt 0 ]; then
		( set -x && source $tmpScript )
	else
		source $tmpScript
	fi

	# Hooks are different. Nix editor doesn't know how to poke those in-between '' blocks.
	local hookScript
	hookScript=$(mkTempFile)
	local tmpFloxNix
	tmpFloxNix=$(mkTempFile)
	$invoke_dasel -w json -f $currentGenDir/manifest.toml | \
		$invoke_jq -r '(.hooks//{}) | to_entries | map(.value | gsub("\n"; "; "))[]' > $hookScript
	$invoke_awk "{print} /hook = / {system(\"cat $hookScript\")}" $nextGenDir/pkgs/default/flox.nix > $tmpFloxNix
	$_mv -f $tmpFloxNix $nextGenDir/pkgs/default/flox.nix

	$_git -C $repoDir add $nextGen/pkgs/default/flox.nix

	local envPackage
	if ! envPackage=$($invoke_nix build --impure --no-link --print-out-paths "$nextGenDir#.floxEnvs.$environmentSystem.default"); then
		error "failed to install packages: ${pkgArgs[@]}" < /dev/null
	fi

	$_jq . --sort-keys $envPackage/catalog.json > $nextGenDir/pkgs/default/catalog.json
	$_jq . --sort-keys $envPackage/manifest.json > $nextGenDir/manifest.json
	$_git -C $repoDir add $nextGen/pkgs/default/catalog.json
	$_git -C $repoDir add $nextGen/manifest.json

	local resultCommitTransaction
	result=$(commitTransaction temporaryAssert008Schema $environment $repoDir $envPackage \
		"$USER converted to 0.0.8 floxmeta schema" 2 \
		"$me automatic conversion")

	warn "Conversion complete. Please re-run command."
	exit 0
}
# /XXX

# XXX TEMPORARY function to rename "$name{,-*-link}" -> "$system.$name{,-*-link}"
#     **Delete after 20230222**
function temporaryAssert009LinkLayout() {
	trace "$@"
	local environment="$1"; shift
	# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
	eval $(decodeEnvironment "$environment")
	# The alias is either "owner/name" or "name" based on the owner, so
	# we can't use that. Instead construct our own fully-qualified
	# name by removing the system from environmentName.
	local environmentBasename="${environmentName/$environmentSystem\./}"
	for i in ${environmentParentDir}/${environmentBasename} ${environmentParentDir}/${environmentBasename}-*-link; do
		if [ -L "$i" ]; then
			local x
			x=$($_readlink "$i")
			case "$x" in
			$environmentSystem.$environmentBasename*)
				# Already renamed, all good.
				: ;;
			$environmentBasename-*-link|/nix/store/*)
				# Old link - rename and leave forwarding link in its place.
				local y="${environmentSystem}.$($_basename $i)"
				if [ -L "${environmentParentDir}/$y" ]; then
					$_rm "$i"
				else
					$_mv "$i" "${environmentParentDir}/$y"
				fi
				$_ln -s "$y" "$i"
				;;
			*)
				warn "cruft detected - please remove: '$i'"
				;;
			esac
		fi
	done
}
# /XXX

#
# gitCheckout($repoDir,$branch)
#
function gitCheckout() {
	trace "$@"
	local repoDir="$1"; shift
	local branch="$1"; shift
	[ -d "$repoDir" ] || gitInitFloxmeta "$repoDir"

	# Confirm or checkout the desired branch.
	local currentBranch=
	if [ -d "$repoDir" ]; then
		currentBranch=$($_git -C "$repoDir" branch --show-current)
	fi
	[ "$currentBranch" = "$branch" ] || {
		if $_git -C "$repoDir" show-ref --quiet refs/heads/"$branch"; then
			$_git -C "$repoDir" checkout --quiet "$branch"
		else
			$_git -C "$repoDir" checkout --quiet --orphan "$branch"
			$_git -C "$repoDir" ls-files | $_xargs --no-run-if-empty $_git -C "$repoDir" rm --quiet -f
			# A commit is needed in order to make the branch visible.
			$_git -C "$repoDir" commit --quiet --allow-empty \
				-m "$USER created profile"
		fi
	}
}

# githubHelperGit()
#
# Invokes git in provided directory with github helper configured.
function githubHelperGit() {
	trace "$@"
	# For github.com specifically, set authentication helper.
	$invoke_git \
		-c "credential.https://github.com.helper=!$_gh auth git-credential" "$@"
}

function metaGit() {
	trace "$@"
	local environment="$1"; shift
	# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
	eval $(decodeEnvironment "$environment")

	# First verify that the clone is not out of date and check
	# out requested branch.
	gitCheckout "$environmentMetaDir" "$branchName"

	githubHelperGit -C "$environmentMetaDir" "$@"
}

# Performs a 'git show branch:file' for the purpose of fishing
# out a file revision without checking out the branch.
function metaGitShow() {
	trace "$@"
	local environment="$1"; shift
	local filename="$1"; shift
	# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
	eval $(decodeEnvironment "$environment")

	# First assert the relevant branch exists.
	if $_git -C "$environmentMetaDir" show-ref --quiet refs/heads/"$branchName"; then
		$invoke_git -C "$environmentMetaDir" show "${branchName}:${filename}"
	else
		error "environment '$environmentOwner/$environmentName' not found for system '$environmentSystem'" < /dev/null
	fi
}

#
# syncEnvironment($environment)
#
function syncEnvironment() {
	trace "$@"
	local environment="$1"; shift
	# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
	eval $(decodeEnvironment "$environment")
	local environmentRealDir
	environmentRealDir=$($_readlink -f $environmentParentDir)

	# Create shared clone for performing work.
	local workDir
	workDir=$(mkTempDir)
	beginTransaction "$environment" "$workDir" 0

	# Run snippet to generate links using data from metadata repo.
	$_mkdir -v -p "$environmentRealDir" 2>&1 | $_sed -e "s/[^:]*:/${me}:/"

	# Invoking the following autogenerated code snippet will:
	# 1. build all the packages in a [nix] profile
	# 2. build the [nix] profile package itself
	# 3. create the GCroot symlinks and top generational symlink
	local snippet
	snippet=$(environmentRegistry "$workDir" "$environment" syncGenerations)
	eval "$snippet" || true

	# FIXME REFACTOR based on detecting actual change.
	[ -z "$_cline" ] || metaGit "$environment" add "metadata.json"
}

function commitMessage() {
	trace "$@"
	local environment="$1"; shift
	# may be empty
	local startGenPath="$1"; shift
	local endGenPath="$1"; shift
	local logMessage="$1"; shift
	local invocation="${@}"
	# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
	eval $(decodeEnvironment "$environment")

	#
	# Now we'd like to include a "diff" of the closures for the log.
	# Nix has rich functionality in this regard but with awkward usage:
	#
	# 1. `nix store diff-closures` has the right usage semantics because
	#    it allows you to specify two profile paths, but it reports more
	#    detail than we're looking for.
	# 2. `nix profile history` gives us the information we're looking for
	#    but expects a linear progression of generations only and won't
	#    report differences between arbitrary generations. It also embeds
	#    color characters in the output and doesn't honor the (mandatory)
	#    `--no-colors` flag. And ... it gives flake references that we
	#    need to convert back to floxpkgs package names.
	#
	# ... so, we mock up a tmpDir with the qualities of #2 above.
	# Not fun but better than nothing.
	#
	local tmpDir
	tmpDir=$(mkTempDir)
	# `nix profile history` requires generations to be in sequential
	# order, so for the purpose of this invocation we set the generations
	# as 1 and 2 if both are defined, or 1 if there is only one generation.
	local myEndGen=
	if [ -n "$startGenPath" ]; then
		$invoke_ln -s "$startGenPath" $tmpDir/${environmentName}-1-link
		myEndGen=2
	else
		myEndGen=1
	fi
	$invoke_ln -s "$endGenPath" $tmpDir/${environmentName}-${myEndGen}-link
	$invoke_ln -s ${environmentName}-${myEndGen}-link $tmpDir/${environmentName}

	local _cline
	$_nix profile history --profile $tmpDir/${environmentName} | $_ansifilter --text | \
		$_awk '\
			BEGIN {p=0} \
			/^  flake:/ {if (p==1) {print $0}} \
			/^Version '${myEndGen}' / {p=1}' | \
		while read _cline
		do
			local flakeref
			flakeref=$(echo "$_cline" | $_cut -d: -f1,2)
			local detail
			detail=$(echo "$_cline" | $_cut -d: -f3-)
			local floxpkg
			floxpkg=$(manifest $environment/manifest.json flakerefToFloxpkg "$flakeref")
			echo "  ${floxpkg}:${detail}"
		done > $tmpDir/commitMessageBody

	if [[ "$logMessage" =~ " upgraded "$ ]]; then
		# When doing an upgrade of everything we don't know what we're
		# upgrading until after its finished. Take this opportunity to
		# replace that message.
		logMessage="${logMessage}$($_cut -d: -f1 $tmpDir/commitMessageBody | $_xargs)"
	fi

	# Actually print log message out to STDOUT.
	cat <<EOF
$logMessage

${invocation[@]}
EOF
	$_cat $tmpDir/commitMessageBody

	# Clean up.
	$_rm -f \
		$tmpDir/"${environmentName}-1-link" \
		$tmpDir/"${environmentName}-2-link" \
		$tmpDir/"${environmentName}" \
		$tmpDir/commitMessageBody
	$_rmdir $tmpDir
}

function checkGhAuth {
	trace "$@"
	local hostname="$1"; shift
	# Repeat login attempts until we're successfully logged in.
	while ! $_gh auth status -h $hostname >/dev/null 2>&1; do
		initialGreeting
		warn "Invoking 'gh auth login -h $hostname'"
		$_gh auth login -h $hostname
		info ""
	done
}

function getUsernameFromGhAuth {
	trace "$@"
	local hostname="$1"; shift
	# Get github username from gh data, if known.
	[ -s "$XDG_CONFIG_HOME/gh/hosts.yml" ]
	$_dasel -f "$XDG_CONFIG_HOME/gh/hosts.yml" "${hostname//./\\.}.user"
}

#
# promptMetaOrigin()
#
# Guides user through the process of prompting for and/or creating
# an origin for their floxmeta repository.
#
function promptMetaOrigin() {
	trace "$@"

	local server organization defaultOrigin origin

	echo 1>&2
	echo "flox uses git to store and exchange metadata between users and machines." 1>&2
	server=$(
		multChoice "Where would you like to host your 'floxmeta' repository?" \
			"git server" "github.com" "gitlab.com" "bitbucket.org" "other"
	)

	case "$server" in
	github.com)
		echo "Great, let's start by getting you logged into $server." 1>&2
		# For github.com only, use the `gh` CLI to make things easy.
		checkGhAuth $server
		if organization=$(getUsernameFromGhAuth $server); then
			echo "Success! You are logged into $server as $organization." 1>&2
		else
			echo "Hmmm ... could not log you into $server. No problem, we can find another way." 1>&2
		fi
		;;
	other)
		read -e -p "git server for storing profile metadata: " server
		;;
	esac

	[ -n "$organization" ] ||
		read -e -p "organization (or username) on $server for creating the 'floxmeta' repository: " organization

	local protocol
	# TODO support ssh+git, but only support https for now, since we use https when
	# we use the gh CLI
	# protocol=$(
	# 	multChoice "What is your preferred protocol for Git operations?" \
	# 		"protocol" "https" "ssh+git"
	# )
	protocol="https"

	case "$protocol" in
	https)
		defaultURL="https://$server/"
		;;
	ssh+git)
		defaultURL="git+ssh://git@$server/"
		;;
	esac

	# Take 'floxmeta' repo name from environment, if defined. Primarily used
	# for testing repo creation, because you cannot simply rename a repo
	# without GitHub helpfully redirecting requests to the renamed repo.
	local repoName="${FLOXMETA_REPO_NAME:-floxmeta}"
	echo "$defaultURL$organization/$repoName"
}

#
# rewriteURLs()
#
# Function to inspect the entirety of a floxmeta repository and rewrite
# any/all URLs that reference the local disk to instead point to the new
# git remote home.
#
function rewriteURLs() {
	trace "$@"
	# TODO once we've finalised the self-referential TOML->environment renderer.
	# Manifests won't contain any references to the floxmeta repository until then.
	return 0
}

#
# getSetOrigin($environment)
#
function getSetOrigin() {
	trace "$@"
	local environment="$1"; shift
	# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
	eval $(decodeEnvironment "$environment")

	# Check to see if the origin is already set.
	local origin
	origin=$([ -d "$environmentMetaDir" ] && $_git -C "$environmentMetaDir" \
		"config" "--get" "remote.origin.url" || true)
	if [ -z "$origin" ]; then

		# Infer/set origin using a variety of information.
		local repoName="${FLOXMETA_REPO_NAME:-floxmeta}"
		if [ "$environmentOwner" == "flox" -o "$environmentOwner" == "flox-examples" ]; then
			# We got this.
			origin="https://github.com/$environmentOwner/floxmeta"
		elif [ $interactive -eq 1 ]; then
			local defaultOrigin
			if [ "$environmentOwner" == "local" ]; then
				defaultOrigin=$(promptMetaOrigin)
			else
				# Strange to have a profile on disk in a named without a
				# remote origin. Prompt user to confirm floxmeta repo on
				# github.
				defaultOrigin="${git_base_url/+ssh/}$environmentOwner/$repoName"
			fi
			echo 1>&2
			read -e \
				-p "confirm git URL for storing profile metadata: " \
				-i "$defaultOrigin" origin
		else
			if [ "$environmentOwner" == "local" ]; then
				# Used primarily for testing; provide default floxmeta origin
				# based on GitHub handle observed by `gh` client.
				local ghAuthHandle
				if ghAuthHandle=$($_gh auth status |& $_awk '/Logged in to github.com as/ {print $7}'); then
					origin="${git_base_url/+ssh/}$ghAuthHandle/$repoName"
				else
					# No chance to discover origin; just create repo and return empty origin.
					[ -d "$environmentMetaDir" ] || gitInitFloxmeta "$environmentMetaDir"
					return 0
				fi
			else
				origin="${git_base_url/+ssh/}$environmentOwner/$repoName"
			fi
		fi

		# A few final cleanup steps.
		if [ "$environmentOwner" == "local" ]; then
			local newEnvironmentOwner
			newEnvironmentOwner=$($_dirname $origin); newEnvironmentOwner=${newEnvironmentOwner/*[:\/]/} # XXX hack

			# rename .cache/flox/meta/{local -> owner} &&
			#   replace with symlink from local -> owner
			# use .cache/flox/meta/owner as environmentMetaDir going forward (only for this function though!)
			if [ -d "$FLOX_META/$newEnvironmentOwner" ]; then
				warn "moving profile metadata directory $FLOX_META/$newEnvironmentOwner out of the way"
				$invoke_mv --verbose $FLOX_META/$newEnvironmentOwner{,.$$}
			fi
			if [ -d "$FLOX_META/local" ]; then
				$invoke_mv "$FLOX_META/local" "$FLOX_META/$newEnvironmentOwner"
			fi
			$invoke_ln -s -f $newEnvironmentOwner "$FLOX_META/local"
			environmentMetaDir="$FLOX_META/$newEnvironmentOwner"

			# rename .local/share/flox/environments/{local -> owner}
			#   replace with symlink from local -> owner
			if [ -d "$FLOX_ENVIRONMENTS/$newEnvironmentOwner" ]; then
				warn "moving environment directory $FLOX_ENVIRONMENTS/$newEnvironmentOwner out of the way"
				$invoke_mv --verbose $FLOX_ENVIRONMENTS/$newEnvironmentOwner{,.$$}
			fi
			if [ -d "$FLOX_ENVIRONMENTS/local" ]; then
				$invoke_mv "$FLOX_ENVIRONMENTS/local" "$FLOX_ENVIRONMENTS/$newEnvironmentOwner"
			fi
			$invoke_ln -s -f $newEnvironmentOwner "$FLOX_ENVIRONMENTS/local"

			# perform single commit rewriting all URL references to refer to new home of floxmeta repo
			rewriteURLs "$FLOX_ENVIRONMENTS/local" "$origin"
		fi

		[ -d "$environmentMetaDir" ] || gitInitFloxmeta "$environmentMetaDir"
		$invoke_git -C "$environmentMetaDir" "remote" "add" "origin" "$origin"
	fi

	ensureGHRepoExists "$origin" private "https://github.com/flox/floxmeta-template.git"
	echo "$origin"
}

#
# beginTransaction($environment, $workDir, $createBranch)
#
# This function creates an ephemeral clone for staging commits to
# a floxmeta repository.
#
function beginTransaction() {
	trace "$@"
	local environment="$1"; shift
	local workDir="$1"; shift
	local -i createBranch="$1"; shift
	# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
	eval $(decodeEnvironment "$environment")

	# If this is a project environment there will be no $environmentMetaDir.
	# Create a simulated generation environment so that we don't have to
	# create project-specific versions of all the calling functions.
	if [ -z "$environmentMetaDir" ]; then
		# Create a fake environmentMetaDir.
		environmentMetaDir=$(mkTempDir)
		gitInitFloxmeta "$environmentMetaDir"

		# Create an ephemeral clone in $workDir.
		$invoke_git clone --quiet --shared "$environmentMetaDir" $workDir

		# Use registry function to initialize metadata.json.
		registry "$workDir/metadata.json" 1 set currentGen 1
		registry "$workDir/metadata.json" 1 setNumber generations 1 version 2
		if [ -L "$environmentBaseDir" ]; then
			local oldEnvironmentPath="$($_readlink "$environmentBaseDir")"
			registry "$workDir/metadata.json" 1 set generations 1 path "$oldEnvironmentPath"
		fi

		# Copy existing flox.nix or create from templateFloxEnv.
		$_mkdir "$workDir/1"
		$invoke_ln -s 1 "$workDir/current"
		$_mkdir "$workDir/current/pkgs"
		if [ -f "$floxNixDir/flox.nix" ]; then
			$_mkdir "$workDir/current/pkgs/default"
			$_cp "$floxNixDir/flox.nix" "$workDir/current/pkgs/default/flox.nix"
			[ ! -f "$floxNixDir/catalog.json" ] ||
				$_cp "$floxNixDir/catalog.json" "$workDir/current/pkgs/default/catalog.json"
			$_cp --no-preserve=mode $_lib/templateFloxEnv/pkgs/default/default.nix "$workDir/current/pkgs/default/default.nix"
		else
			$_cp --no-preserve=mode -rT $_lib/templateFloxEnv "$workDir/current/."
		fi

		# Link next generation.
		$_mkdir -p "$workDir/2"; $_ln -s 2 "$workDir/next"

		# Simulation complete; bid a hasty retreat.
		return 0
	fi

	# Verify that $environmentMetaDir/local exists either as a directory
	# or as a symlink to another directory.
	if [ ! -d "$environmentMetaDir" ]; then
		if [ -L "$environmentMetaDir" ]; then
			error "damaged symbolic link: $environmentMetaDir" < /dev/null
		else
			gitInitFloxmeta "$environmentMetaDir"
		fi
	fi

	# Perform a fetch to get remote data into sync.
	if $invoke_git -C "$environmentMetaDir" show-ref --quiet refs/remotes/origin/HEAD; then
		githubHelperGit -C "$environmentMetaDir" fetch origin
	fi

	# Create an ephemeral clone.
	$invoke_git clone --quiet --shared "$environmentMetaDir" $workDir

	# Check out the relevant branch. Can be complicated in the event
	# that this is the first pull of a brand-new branch.
	if $invoke_git -C "$workDir" show-ref --quiet refs/heads/"$branchName"; then
		$invoke_git -C "$workDir" checkout --quiet "$branchName"
	elif $invoke_git -C "$workDir" show-ref --quiet refs/remotes/origin/"$branchName"; then
		$invoke_git -C "$workDir" checkout --quiet --track origin/"$branchName"
	elif [ $createBranch -eq 1 ]; then
		$invoke_git -C "$workDir" checkout --quiet --orphan "$branchName"
		$invoke_git -C "$workDir" ls-files | $_xargs --no-run-if-empty $_git -C "$workDir" rm --quiet -f
		# A commit is needed in order to make the branch visible.
		$invoke_git -C "$workDir" commit --quiet --allow-empty \
			-m "$USER created environment $environmentName ($environmentSystem)"
	else
		error "environment $environmentAlias ($environmentSystem) does not exist" < /dev/null
	fi

	# XXX Temporary covering transition from 0.0.6 -> 0.0.7
	temporaryAssert007Schema "$workDir"
	# /XXX

	# Any function calling this one will probably be wanting to make
	# some sort of change that will generate a new generation, so take
	# this opportunity to identify the current and next generations
	# and drop in helper symlinks pointing to the "current" and "next"
	# generations to make it easy for calling functions to make changes.
	# (But don't add them to the git index.)

	# Record starting generation.
	local -i startGen
	startGen=$(registry "$workDir/metadata.json" 1 currentGen)
	if [ $startGen -gt 0 ]; then
		$invoke_ln -s $startGen "$workDir/current"
	fi

	# Calculate next available generation. Note this is _not_ just
	# (startGen + 1), but rather (max(generations) + 1) as recorded
	# in the environment registry. (We're no longer using symlinks
	# to record this in the floxmeta repo.)
	local -i nextGen
	nextGen=$(registry "$workDir/metadata.json" 1 nextGen)
	$invoke_mkdir -p $workDir/$nextGen
	$invoke_ln -s $nextGen $workDir/next

	# XXX Temporary covering transition from 0.0.7 -> 0.0.8
	temporaryAssert008Schema "$environment" "$workDir"
	# /XXX
}

#
# cmpV1Environments(env1, env2)
#
# Examines two V1 environments to determine if they are different.
# Like cmp(1) itself, will return nonzero when there are changes
# or 0 when they are substantively the same.
#
function cmpV1Environments() {
	local env1="$1"; shift
	local env2="$1"; shift
	# $env1 (the new gen) has been determined to be a V1, but $env2
	# that it is replacing may be any version, which may or may not
	# have a manifest.json file to inspect. First test that both
	# environments have manifest.json files to be compared.
	if [ -f "$env1/manifest.json" -a -f "$env2/manifest.json" ]; then
		$invoke_jq -n -f $_lib/diff-manifests.jq \
			--slurpfile m1 "$env1/manifest.json" \
			--slurpfile m2 "$env2/manifest.json" || return 1
	else
		return 1
	fi
	return 0
}

#
# cmpEnvironments(version, env1, env2)
#
function cmpEnvironments() {
	trace "$@"
	local version="$1"; shift
	local env1="$1"; shift
	local env2="$1"; shift
	[ "$env1" = "$env2" ] || case $version in
		1)
			cmpV1Environments "$env1" "$env2" || return 1
			;;
		2)
			# floxEnv environments are referenced by way of helper symlinks.
			# Use realpath to follow those links and compare the packages.
			local realpathEnv1
			realpathEnv1=$($_realpath "$env1")
			local realpathEnv2
			realpathEnv2=$($_realpath "$env2")
			[ "$realpathEnv1" = "$realpathEnv2" ] || return 1
			;;
		esac
	return 0
}

#
# commitTransaction($environment, $workDir, $logMessage)
#
# This function completes the process of committing updates to
# a floxmeta repository from an ephemeral clone.
#
function commitTransaction() {
	trace "$@"
	local action="$1"; shift
	local environment="$1"; shift
	local workDir="$1"; shift
	local environmentPackage="$1"; shift
	local logMessage="$1"; shift
	local nextGenVersion="$1"; shift
	local invocation="${@}"
	local result=""
	# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
	eval $(decodeEnvironment "$environment")

	# If this is a project environment there will be no $environmentMetaDir,
	# and correspondingly nothing to commit or push. The only thing we need
	# to do in this instance is update the activation link and bid a hasty
	# retreat.
	if [ -z "$environmentMetaDir" ]; then
		if $_cmp -s "$workDir/next/pkgs/default/flox.nix" "$protoPkgDir/flox.nix";
		then
			result="project-environment-no-changes"
		else
			result="project-environment-modified"
		fi

		$invoke_nix_store --add-root "$environmentBaseDir" -r $environmentPackage >/dev/null
		$invoke_cp "$workDir/next/pkgs/default/flox.nix" "$floxNixDir/flox.nix"
		$invoke_cp "$workDir/next/pkgs/default/catalog.json" "$floxNixDir/catalog.json"

		echo -n $result
		return 0
	fi

	# Glean current and next generations from clone.
	local -i currentGen
	currentGen=$($_readlink $workDir/current || echo 0)
	local -i nextGen
	nextGen=$($_readlink $workDir/next)

	# XXX temporary: as we change to version 0.0.9 the layout of environment
	# links changes to embed the system type. Take this opportunity to rename
	# those links if they exist.
	temporaryAssert009LinkLayout "$environment"

	# Activate the new generation just as Nix would have done.
	# First check to see if the environment has actually changed,
	# and if not then return immediately.
	local oldEnvPackage
	if [ -e "$environment" ]; then
		oldEnvPackage=$(registry "$workDir/metadata.json" 1 get generations $currentGen path)
	fi

	# Check to see if there has been a change.
	if [ -n "$oldEnvPackage" ] && cmpEnvironments $nextGenVersion "$environmentPackage" "$oldEnvPackage"; then
		# The rendered environments are the same, which means this is a no-op
		# except in the case where someone has done `flox edit` and changed
		# the flox.nix file.
		if [ "$action" != "edit" ] || $_cmp --quiet "$workDir/$currentGen/pkgs/default/flox.nix" "$workDir/$nextGen/pkgs/default/flox.nix"; then
			if [ $verbose -ge 1 ]; then
				warn "No environment changes detected .. exiting"
			fi
			echo -n "named-environment-no-changes"
			return 0
		fi
	fi

	# Update the floxmeta registry to record the new generation.
	registry "$workDir/metadata.json" 1 set currentGen $nextGen

	# Figure out if we're creating or switching to an existing generation.
	local createdOrSwitchedTo="created"
	if $invoke_jq -e --arg gen $nextGen '.generations | has($gen)' $workDir/metadata.json >/dev/null; then
		result="named-environment-switch-to-generation"
		createdOrSwitchedTo="switched to"
	else
		result="named-environment-created-generation"
		# Update environment metadata with new end generation information.
		registry "$workDir/metadata.json" 1 set generations \
			${nextGen} path $environmentPackage
		registry "$workDir/metadata.json" 1 addArray generations \
			${nextGen} logMessage "$logMessage"
		registry "$workDir/metadata.json" 1 setNumber generations \
			${nextGen} created "$now"
		registry "$workDir/metadata.json" 1 setNumber generations \
			${nextGen} lastActive "$now"
		registry "$workDir/metadata.json" 1 setNumber generations \
			${nextGen} version $nextGenVersion
	fi

	# Also update lastActive time for current generation, if known.
	[ $currentGen -eq 0 ] || \
		registry "$workDir/metadata.json" 1 setNumber generations \
			$currentGen lastActive "$now"

	# Mark the metadata.json file to be included with the commit.
	$invoke_git -C $workDir add "metadata.json"

	# Now that metadata is recorded, actually put the change
	# into effect. Must be done before calling commitMessage().
	if [ "$createdOrSwitchedTo" = "created" ]; then
		$invoke_nix_store --add-root "${environment}-${nextGen}-link" \
			-r $environmentPackage >/dev/null
	fi
	$invoke_rm -f $environment
	$invoke_ln -s "${environmentName}-${nextGen}-link" $environment

	# Detect version and act accordingly.
	local -i currentGenVersion
	if ! currentGenVersion=$(registry $workDir/metadata.json 1 get generations "$currentGen" version); then
		currentGenVersion=1
	fi
	# Unification TODO: use catalog.json instead of relying on manifest.json
	local message
	message=$(commitMessage \
		"$environment" "$oldEnvPackage" "$environmentPackage" \
		"$logMessage" "${invocation[@]}")

	$invoke_git -C $workDir commit -m "$message" --quiet
	$invoke_git -C $workDir push --quiet --set-upstream origin $branchName

	# Tom's feature: teach a man to fish with (-v|--verbose)
	if [ $verbose -ge 1 -a $currentGenVersion -eq 2 -a $nextGenVersion -eq 2 ]; then
		$invoke_git -C $workDir diff HEAD:{$currentGen,$nextGen}/pkgs/default/flox.nix
		warn "$createdOrSwitchedTo generation $nextGen"
	fi

	echo -n $result
}

#
# listEnvironments($system)
#
function listEnvironments() {
	trace "$@"
	local system="$1"; shift
	local environmentMetaDir="$1"; shift
	local environmentOwner
	environmentOwner=$($_basename $environmentMetaDir)

	# Quick sanity check .. is this a git repo?
	$_git -C "$environmentMetaDir" rev-parse 2> /dev/null || \
		error "not a git clone? Please remove: $environmentMetaDir" < /dev/null

	# Start by updating all remotes in the clone dir.
	githubHelperGit -C $environmentMetaDir fetch --quiet --all

	# Derive all known branches. Recall branches will be of the form:
	#   remotes/origin/x86_64-linux.default
	#   remotes/upstream/x86_64-linux.default
	local -A _branches
	local -A _local
	local -A _origin
	local -a _cline
	. <($invoke_git -C $environmentMetaDir branch -av | $_sed 's/^\*//' | while read -a _cline
		do
			_remote=$($_dirname "${_cline[0]}")
			_branch=$($_basename "${_cline[0]}")
			if [[ "$_branch" =~ ^$system.* ]]; then
				_revision="${_cline[1]}"
				case "$_remote" in
				"remotes/origin")
					echo "_branches[\"$_branch\"]=1"
					echo "_origin[\"$_branch\"]=\"$_revision\""
					;;
				"remotes/*")
					warn "unexpected remote '$_remote' in $environmentMetaDir clone ... ignoring"
					;;
				*)
					echo "_branches[\"$_branch\"]=1"
					echo "_local[\"$_branch\"]=\"$_revision\""
					;;
				esac
			fi
		done
	)

	# Iterate over branches printing out everything we know.
	for _branch in $(echo ${!_branches[@]} | $_xargs -n 1 | $_sort); do
		local __local="${_local[$_branch]}"
		local __origin="${_origin[$_branch]}"
		local __commit="unknown"
		local -i __printCommit=0
		local __generation="unknown"
		local __name=${_branch##*.}
		local __path="$FLOX_ENVIRONMENTS/$environmentOwner/$__name"
		local __alias="$environmentOwner/$__name"
		local __localEnvironmentOwner="local"
		if [ -L "$FLOX_ENVIRONMENTS/local" ]; then
			__localEnvironmentOwner=$($_readlink "$FLOX_ENVIRONMENTS/local")
		fi
		if [ "$__localEnvironmentOwner" = "$environmentOwner" ]; then
			__alias="$__name"
		fi
		if [ -n "$__local" ]; then
			local __metadata
			__metadata=$(mkTempFile)
			if $invoke_git -C $environmentMetaDir show $__local:metadata.json > $__metadata 2>/dev/null; then
				__commit="$__local"
				__generation=$($invoke_jq -r .currentGen $__metadata)
			fi
		fi
		if [ -n "$__origin" -a "$__origin" != "$__local" ]; then
			local __metadata
			__metadata=$(mkTempFile)
			if $invoke_git -C $environmentMetaDir show $__origin:metadata.json > $__metadata 2>/dev/null; then
				__commit="$__commit (remote $__origin)"
				__printCommit=1
				__generation=$($invoke_jq -r .currentGen $__metadata)
			fi
		fi
		$_cat <<EOF
$environmentOwner/$__name
    Alias     $__alias
    System    $system
    Path      $FLOX_ENVIRONMENTS/$environmentOwner/$__name
    Curr Gen  $__generation
EOF
		if [ $verbose -eq 0 ]; then
			[ $__printCommit -eq 0 ] || echo "    Commit    $__commit"
		else
			$_cat <<EOF
    Branch    $environmentOwner/$_branch
    Commit    $__commit
EOF
		fi
		echo ""
	done
}

#
# doAutoUpdate($environment)
#
# Decide whether to attempt an auto-update of the provided environment.
# Returns 0 (never), 1 (prompt), or 2 (pull without prompting) depending
# on environment variables, name of environment, and (eventually) other
# criteria.
#
function doAutoUpdate() {
	trace "$@"
	local environment="$1"; shift
	case "$FLOX_AUTOUPDATE" in
	0|1|2) echo "$FLOX_AUTOUPDATE";;
	"") echo 1;;
	*)
		warn "ignoring invalid value '$FLOX_AUTOUPDATE' for '\$FLOX_AUTOUPDATE'"
		echo 1;;
	esac
}

#
# updateAvailable($environment)
#
# Checks to see if origin/branchname is ahead of the local branchname,
# and if so echoes the generation number of the upstream version, and
# otherwise echoes 0 to indicate that the generations are the same.
#
function updateAvailable() {
	trace "$@"
	local environment="$1"; shift

	# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
	eval $(decodeEnvironment "$environment")

	# First calculate current generation number.
	if [ -d "$environmentMetaDir" ]; then
		if $_git -C "$environmentMetaDir" show-ref --quiet refs/heads/"$branchName" 2>/dev/null; then
			local tmpfile
			tmpfile=$(mkTempFile)
			if $invoke_git -C "$environmentMetaDir" show "${branchName}:metadata.json" >$tmpfile 2>/dev/null; then
				local -i currentGen
				if currentGen=$(registry $tmpfile 1 get currentGen); then
					# If that worked then calculate generation number upstream.
					if $_git -C "$environmentMetaDir" show-ref --quiet refs/remotes/origin/"$branchName" 2>/dev/null; then
						if $invoke_git -C "$environmentMetaDir" show "origin/${branchName}:metadata.json" >$tmpfile 2>/dev/null; then
							local -i currentOriginGen
							if currentOriginGen=$(registry $tmpfile 1 get currentGen); then
								if [ $currentGen -lt $currentOriginGen ]; then
									echo $currentOriginGen
									return 0
								fi
							fi
						fi
					fi
				fi
			fi
		fi
	fi
	echo 0
}

#
# trailingAsyncFetch()
#
# Perform a sequential "trailing fetch" of the floxmeta repositories
# for the set of environments passed in "$@".
#
function _trailingAsyncFetch() {
	trace "$@"
	for metaDir in "$@"; do
		githubHelperGit -C "$metaDir" fetch origin || :
	done
	exit 0
}
function trailingAsyncFetch() {
	trace "$@"
	[ $# -gt 0 ] || return 0
	local -A trailingAsyncFetchMetaDirs
	for environment in "$@"; do
		# set $branchName,$floxNixDir,$environment{Name,Alias,Owner,System,BaseDir,BinDir,ParentDir,MetaDir}
		eval $(decodeEnvironment "$environment")
		# $environmentMetaDir will be blank for project environments.
		if [ -n "$environmentMetaDir" ]; then
			trailingAsyncFetchMetaDirs["$environmentMetaDir"]=1
		fi
	done
	# Make every effort to stay hidden in the background unless debugging.
	if [ $debug -gt 0 ]; then
		( _trailingAsyncFetch "${!trailingAsyncFetchMetaDirs[@]}" </dev/null & )
	else
		( _trailingAsyncFetch "${!trailingAsyncFetchMetaDirs[@]}" </dev/null & ) >/dev/null 2>&1
	fi
}

# vim:ts=4:noet:syntax=bash
