# Flox environment activation script.
#
# Variables provided by nix (`.#flox-activate`):
#   _coreutils: path to GNU coreutils package
#   _gnused:    path to GNU sed package
#   _procps:    path to procps package
#   _zdotdir:   path to zsh dotfiles directory

export _FLOX_PKGDB_VERBOSITY="${_FLOX_PKGDB_VERBOSITY:-0}"
[ "$_FLOX_PKGDB_VERBOSITY" -eq 0 ] || set -x


# TODO: add getopt arg parser for following args:
# -c "<cmd> <args>": specify exact command args to pass to shell
# --turbo: invoke commands directly without involving userShell
# --noprofile: do not source `[profile]` scripts

# Set FLOX_ENV as the path by which all flox scripts can make reference to
# the environment to which they belong. Use this to define the path to the
# activation scripts directory.
# TODO: reconcile with CLI which should be setting this. We must override
#       the value coming from the CLI for now because it won't be set for
#       container invocations, and it would have the incorrect value for
#       nested flox activations.
_FLOX_ENV="$( $_coreutils/bin/dirname -- "${BASH_SOURCE[0]}" )"
if [ -n "$FLOX_ENV" -a "$FLOX_ENV" != "$_FLOX_ENV" ]; then
  echo "WARN: detected change in FLOX_ENV: $FLOX_ENV -> $_FLOX_ENV" >&2
fi
export FLOX_ENV="$_FLOX_ENV"

# The rust CLI contains sophisticated logic to set $FLOX_SHELL based on the
# process listening on STDOUT, but that won't happen when activating from
# the top-level activation script, so fall back to $SHELL as a default.
FLOX_SHELL="${FLOX_SHELL:-$SHELL}"

# Set all other variables derived from FLOX_ENV. We previously did this
# from within the rust CLI but we've moved it to this top-level activation
# script so that it can be invoked without using the flox CLI, e.g. as
# required when invoking the environment from a container entrypoint.

# Identify if this environment has been activated before. If it has,
# then it will appear as an element in the colon-separated FLOX_ENV_DIRS
# variable, and if it hasn't then we'll prepend it to the list and set
# all the other related env variables.
declare -a flox_env_dirs
IFS=: read -ra flox_env_dirs <<< "${FLOX_ENV_DIRS_activate}"
declare -i flox_env_found=0
for d in "${flox_env_dirs[@]}"; do
  if [ "$d" = "$FLOX_ENV" ]; then
    flox_env_found=1
    break
  fi
done
if [ $flox_env_found -eq 0 ]; then

  # First activation of this environment. Snapshot environment to start.
  _start_env="$($_coreutils/bin/mktemp --suffix=.start-env)"
  export | $_coreutils/bin/sort > "$_start_env"

  # Capture PID of this "first" activation. This provides the unique
  # identifier with which to refer to environment variables associated
  # with this environment activation.
  FLOX_ENV_PID="$$"

  # Set environment variables which represent the cumulative layering
  # of flox environments. For the most part this involves prepending
  # to the existing variables of the same name.
  # TODO: reconcile with CLI which should be setting these. Setting
  #       "*_activate" variables to indicate the ones we've seen and
  #       processed on the activate script side, and ultimately also
  #       for testing/comparison against the CLI-maintained equivalents.
  FLOX_ENV_DIRS_activate="$FLOX_ENV${FLOX_ENV_DIRS_activate:+:$FLOX_ENV_DIRS_activate}"
  FLOX_ENV_LIB_DIRS_activate="$FLOX_ENV/lib${FLOX_ENV_LIB_DIRS_activate:+:$FLOX_ENV_LIB_DIRS_activate}"
  FLOX_PROMPT_ENVIRONMENTS_activate="$FLOX_ENV_DESCRIPTION${FLOX_PROMPT_ENVIRONMENTS_activate:+ $FLOX_PROMPT_ENVIRONMENTS_activate}"
  export FLOX_ENV_DIRS_activate FLOX_ENV_LIB_DIRS_activate FLOX_PROMPT_ENVIRONMENTS_activate

  # Process the flox environment customizations, which includes (amongst
  # other things) prepending this environment's bin directory to the PATH.
  if [ -d "$FLOX_ENV/etc/profile.d" ]; then
    declare -a _prof_scripts;
    _prof_scripts=( $(
      cd "$FLOX_ENV/etc/profile.d";
      shopt -s nullglob;
      echo *.sh;
    ) );
    for p in "${_prof_scripts[@]}"; do . "$FLOX_ENV/etc/profile.d/$p"; done
    unset _prof_scripts;
  fi

  # Set static environment variables from the manifest.
  if [ -f "$FLOX_ENV/activate.d/envrc" ]; then
    source "$FLOX_ENV/activate.d/envrc"
  fi

  # Source the hook-on-activate script if it exists.
  if [ -e "$FLOX_ENV/activate.d/hook-on-activate" ]; then
    # Nothing good can come from output printed to stdout in the
    # user-provided hook scripts because these can get interpreted
    # as configuration statements by the "in-place" activation
    # mode. So, we'll redirect stdout to stderr.
    source "$FLOX_ENV/activate.d/hook-on-activate" 1>&2
  fi

  # Capture ending environment.
  _end_env="$($_coreutils/bin/mktemp --suffix=.$FLOX_ENV_PID.end-env)"
  export | $_coreutils/bin/sort > "$_end_env"

  # The userShell initialization scripts that follow have the potential to undo
  # the environment modifications performed above, so we must first calculate
  # all changes made to the environment so far so that we can restore them after
  # the userShell initialization scripts have run. We use the `comm(1)` command
  # to compare the starting and ending environment captures (think of it as a
  # better diff for comparing sorted files), and `sed(1)` to format the output
  # in the best format for use in each language-specific activation script.
  _add_env="$($_coreutils/bin/mktemp --suffix=.$FLOX_ENV_PID.add-env)"
  _del_env="$($_coreutils/bin/mktemp --suffix=.$FLOX_ENV_PID.del-env)"

  # Export tempfile paths for use within shell-specific activation scripts.
  export _add_env _del_env

  # Capture environment variables to _set_ as "key=value" pairs.
  # comm -13: only env declarations unique to `$_end_env` (new declarations)
  $_coreutils/bin/comm -13 "$_start_env" "$_end_env" | \
    $_gnused/bin/sed -e 's/^declare -x //' > $_add_env

  # Capture environment variables to _unset_ as a list of keys.
  # TODO: remove from $_del_env keys set in $_add_env
  $_coreutils/bin/comm -23 "$_start_env" "$_end_env" | \
    $_gnused/bin/sed -e 's/^declare -x //' -e 's/=.*//' > $_del_env

  # Don't need these anymore.
  $_coreutils/bin/rm -f "$_start_env" "$_end_env"

else

  # "Reactivation" of this environment.

  # If we're attempting to launch an interactive shell then just print a
  # message to say that the environment has already been activated.
  if [ -t 1 ] && [ $# -eq 0 ]; then
    echo "ERROR: Environment '$FLOX_ENV_DESCRIPTION' is already active." >&2
    exit 1
  fi

  # Assert that the expected _{add,del}_env variables are present.
  [ -n "$_add_env" -a -n "$_del_env" ] || {
    echo 'ERROR (activate): $_add_env and $_del_env not found in environment' >&2;
    if [ -h "$FLOX_ENV" ]; then
      echo "moving $FLOX_ENV link to $FLOX_ENV.$$ - please try again" >&2;
      $_coreutils/bin/mv $FLOX_ENV $FLOX_ENV.$$
    fi
    exit 1;
  }

  # Replay the environment for the benefit of this shell.
  eval "$($_gnused/bin/sed -e 's/^/unset /' -e 's/$/;/' $_del_env)"
  eval "$($_gnused/bin/sed -e 's/^/export /' -e 's/$/;/' $_add_env)"

fi

# From this point on the activation process depends on the mode:

# 1. "command" mode(s): invoke the user's shell with args that:
#   a. defeat the shell's normal startup scripts
#   b. source the relevant activation script
#   c. invoke the command in one of "stdin" or "-c" modes
if [ $# -gt 0 ]; then
  if [ $# -ne 2 -o "$1" != "-c" ]; then
    # Marshal the provided args into a single safely-quoted string.
    # We use the magic "${@@Q}" parameter transformation to return
    # each element of "$@" as a safely quoted string.
    declare -a cmdarray=()
    cmdarray=("-c" "$(echo "${@@Q}")")
    set -- "${cmdarray[@]}"
  fi
  if [ -n "$FLOX_TURBO" ]; then
    # "turbo command" mode: simply exec the provided command and args
    # without paying the cost of invoking the userShell.
    eval "exec $2"
  fi
  # "-c" command mode: pass both [2] arguments unaltered to shell invocation
  case "$FLOX_SHELL" in
    *bash)
      if [ -n "$FLOX_NO_PROFILES" ]; then
        exec "$FLOX_SHELL" --noprofile --norc "$@"
      else
        if [ -t 1 ]; then
          exec "$FLOX_SHELL" --noprofile --rcfile "$FLOX_ENV/activate.d/bash" "$@"
        else
          # The bash --rcfile option only works for interactive shells
          # so we need to cobble together our own means of sourcing our
          # startup script for non-interactive shells.
          exec "$FLOX_SHELL" --noprofile --norc -s <<< "source $FLOX_ENV/activate.d/bash && $2"
        fi
      fi
      ;;
    *zsh)
      if [ -n "$FLOX_NO_PROFILES" ]; then
        exec "$FLOX_SHELL" -o NO_GLOBAL_RCS -o NO_RCS "$@"
      else
        export FLOX_ORIG_ZDOTDIR="$ZDOTDIR"
        export ZDOTDIR="$_zdotdir"
        export FLOX_ZSH_INIT_SCRIPT="$FLOX_ENV/activate.d/zsh"
        # The "NO_GLOBAL_RCS" option is necessary to prevent zsh from
        # automatically sourcing /etc/zshrc et al.
        exec "$FLOX_SHELL" -o NO_GLOBAL_RCS "$@"
      fi
      ;;
    *)
      echo "Unsupported shell: $FLOX_SHELL" >&2
      exit 1
      ;;
  esac
fi

# 2. "interactive" mode: invoke the user's shell with args that:
#   a. defeat the shell's normal startup scripts
#   b. source the relevant activation script
if [ -t 1 -o -n "$_FLOX_FORCE_INTERACTIVE" ]; then
  case "$FLOX_SHELL" in
    *bash)
      if [ -n "$FLOX_NO_PROFILES" ]; then
        exec "$FLOX_SHELL" --noprofile --norc
      else
        if [ -t 1 ]; then
          exec "$FLOX_SHELL" --noprofile --rcfile "$FLOX_ENV/activate.d/bash"
        else
          # The bash --rcfile option only works for interactive shells
          # so we need to cobble together our own means of sourcing our
          # startup script for non-interactive shells.
          # XXX Is this case even a thing? What's the point of activating with
          #     no command to be invoked and no controlling terminal from which
          #     to issue commands?!? A broken docker experience maybe?!?
          exec "$FLOX_SHELL" --noprofile --norc -s <<< "source $FLOX_ENV/activate.d/bash"
        fi
      fi
      ;;
    *zsh)
      if [ -n "$FLOX_NO_PROFILES" ]; then
        exec "$FLOX_SHELL" -o NO_GLOBAL_RCS -o NO_RCS
      else
        export FLOX_ORIG_ZDOTDIR="$ZDOTDIR"
        export ZDOTDIR="$_zdotdir"
        export FLOX_ZSH_INIT_SCRIPT="$FLOX_ENV/activate.d/zsh"
        # The "NO_GLOBAL_RCS" option is necessary to prevent zsh from
        # automatically sourcing /etc/zshrc et al.
        exec "$FLOX_SHELL" -o NO_GLOBAL_RCS
      fi
      ;;
    *)
      echo "Unsupported shell: $FLOX_SHELL" >&2
      exit 1
      ;;
  esac
fi

# 3. "in-place" mode: emit activation commands in correct shell dialect

# Finish by echoing the contents of the shell-specific activation script.
# N.B. the output of these scripts may be eval'd with backticks which have
# the effect of removing newlines from the output, so we must ensure that
# the output is a valid shell script fragment when represented on a single
# line.
case "$FLOX_SHELL" in
  *bash)
    echo "export FLOX_ENV=\"$FLOX_ENV\";"
    echo "export _FLOX_PKGDB_VERBOSITY=\"$_FLOX_PKGDB_VERBOSITY\";"
    echo "export _add_env=\"$_add_env\";"
    echo "export _del_env=\"$_del_env\";"
    echo "source '$FLOX_ENV/activate.d/bash';"
    ;;
  *zsh)
    echo "export FLOX_ENV=\"$FLOX_ENV\";"
    echo "export _FLOX_PKGDB_VERBOSITY=\"$_FLOX_PKGDB_VERBOSITY\";"
    echo "export FLOX_ORIG_ZDOTDIR=\"$FLOX_ORIG_ZDOTDIR\";"
    echo "export ZDOTDIR=\"$_zdotdir\";"
    echo "export FLOX_ZSH_INIT_SCRIPT=\"$FLOX_ENV/activate.d/zsh\";"
    echo "export _add_env=\"$_add_env\";"
    echo "export _del_env=\"$_del_env\";"
    echo "source '$FLOX_ENV/activate.d/zsh';"
    ;;
  *)
    echo "unsupported shell: $FLOX_SHELL" >&2
    exit 1
    ;;
esac
