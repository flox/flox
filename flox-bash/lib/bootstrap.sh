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
	if [ -t 1 ]; then
		# Interactive mode
		interactive=1

		# Collect the user's express consent to submit telemetry data.
		if [ -z "$FLOX_DISABLE_METRICS" ]; then
			if ! floxMetricsConsent=$(floxUserMetaRegistry get floxMetricsConsent); then
				info ""
				info "flox collects basic usage metrics in order to improve the user experience,"
				info "including a record of the subcommand invoked along with a unique token."
				info "It does not collect any personal information."
				info ""
				if boolPrompt "Do you consent to the collection of basic usage metrics?" "yes"; then
					floxUserMetaRegistry setNumber floxMetricsConsent 1
					info ""
					info "Thank you for helping to improving flox!"
					info ""
				else
					floxUserMetaRegistry setNumber floxMetricsConsent 0
					info ""
					info "Understood. If you change your mind you can change your election"
					info "at any time with the following command: flox config --reset"
					info ""
				fi
				floxMetricsConsent=$(floxUserMetaRegistry get floxMetricsConsent)
			fi
		fi

		# Note whether user has seen various educational/informational messages.
		educatePublish=$(floxUserMetaRegistry get educatePublish) || \
			floxUserMetaRegistry setNumber educatePublish 0

	else

		#
		# Non-interactive mode. Use all defaults if not found in registry.
		#
		if [ -z "$FLOX_DISABLE_METRICS" ]; then
			floxMetricsConsent=$(floxUserMetaRegistry get floxMetricsConsent) || \
				floxMetricsConsent=0
		fi
		# Only educate in interactive mode; setting educatePublish=1
		# means user has been educated.
		educatePublish=1

	fi
}

bootstrap

# vim:ts=4:noet:syntax=bash
