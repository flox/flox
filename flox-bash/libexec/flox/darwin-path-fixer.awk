#
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
# See following URL for more detail:
# https://discourse.floxdev.com/t/losing-part-of-my-shell-environment-when-using-flox-develop/556/2
#
# Usage: echo $PATH | awk -v shellDialect=bash -f path/to/darwin-path-fixer.awk
#

BEGIN {
	if (shellDialect == "bash" || shellDialect == "csh") {
		RS = ":"  # Split input records on ":".
		ORS = ":" # Print output records joined by ":".
	} else if (shellDialect == "fish") { # XXX untested probably wrong
		RS = " "  # Split input records on " ".
		ORS = " " # Print output records joined by " ".
	} else {
		if (shellDialect != "") {
			print "ERROR: unknown shellDialect " shellDialect > "/dev/stderr"
		} else {
			print "ERROR: missing shellDialect argument" > "/dev/stderr"
		}
		print "Usage: awk -v shellDialect=(bash|csh|fish) -f <this script>" > "/dev/stderr"
		exit 1
	}
	# Populate an array of path elements we're looking for.
	split(ENVIRON["FLOX_ACTIVE_ENVIRONMENTS"], bindirs, ":")
	for (i=1; i<=length(bindirs); i++) {
		# With awk simply accessing an associative array index
		# will create it with an empty value.
		activeEnvironmentBinDirs[bindirs[i] "/bin"]
	}
}

{
	sub(/\n$/, "") # trim trailing newlines (from last element)
	if (!($0 in seenBinDirs)) { # Ignore dups as we go.
		seenBinDirs[$0]
		# File into flox/non-flox buckets as we process binDirs.
		if ($0 in activeEnvironmentBinDirs) {
			floxBinDirs[floxBinDirsLen++] = $0
		} else {
			otherBinDirs[otherBinDirsLen++] = $0
		}
	}
}

END {
	# Print flox bindirs first.
	for ( i = 0; i < floxBinDirsLen; i++ )
		print floxBinDirs[i]
	# Print out all but the final bindir in otherBinDirs.
	for ( i = 0; i < (otherBinDirsLen-1); i++ )
		print otherBinDirs[i]
	# Print out final element without a trailing record separator.
	printf("%s\n",otherBinDirs[otherBinDirsLen-1])
}

# vim:ts=4:noet:syntax=awk
