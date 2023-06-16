# Boolean to track whether this is the initial bootstrap.
declare -i _initial_bootstrap=0

declare -i _greeted=0
function initialGreeting {
	trace "$@"
	[ $_initial_bootstrap -eq 1 ] || return 0
	[ $_greeted -eq 0 ] || return 0
	$_cat <<EOF 1>&2

I see you are new to flox! We just need to set up a few things
to get you started ...

EOF
	_greeted=1
}

# Bootstrap the personal metadata.
declare git_base_url="$FLOX_CONF_git_base_url"
declare floxUserMeta
floxUserMeta=$(mkTempFile)
function bootstrap() {
	$_git -C "$userFloxMetaCloneDir" \
		show "$defaultBranch:floxUserMeta.json" >$floxUserMeta 2>/dev/null || _initial_bootstrap=1
	floxUserMetaRegistry get floxClientUUID >/dev/null || \
		floxUserMetaRegistry set floxClientUUID $($_uuid)
	floxClientUUID=$(floxUserMetaRegistry get floxClientUUID)
	if [ -t 0 -a -t 2 ]; then
		# Interactive mode
		interactive=1

		# Note whether user has seen various educational/informational messages.
		educatePublish=$(floxUserMetaRegistry get educatePublish) || \
			floxUserMetaRegistry setNumber educatePublish 0

	else
		# Only educate in interactive mode; setting educatePublish=1
		# means user has been educated.
		educatePublish=1
	fi
}

bootstrap

# vim:ts=4:noet:syntax=bash
