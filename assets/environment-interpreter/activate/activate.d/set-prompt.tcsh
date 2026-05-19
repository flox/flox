$_flox_activate_tracer $_activate_d/set-prompt.tcsh START

# Tweak the (already customized) prompt: add a flox indicator.
if ( $?prompt && "$FLOX_PROMPT_ENVIRONMENTS" != "" ) then
    if ( ! $?FLOX_PROMPT ) then
        set FLOX_PROMPT = "flox"
    endif

    set colorReset = "%{\033[0m%}"
    set colorBold = "%{\033[1m%}"
    set colorPrompt1 = "%{\033[38;5;""$FLOX_PROMPT_COLOR_1""m%}"
    set colorPrompt2 = "%{\033[38;5;""$FLOX_PROMPT_COLOR_2""m%}"
    set _floxPrompt1 = "$colorPrompt1""$FLOX_PROMPT"
    set _floxPrompt2 = "$colorPrompt2""[$FLOX_PROMPT_ENVIRONMENTS]"

    if $?NO_COLOR then
        set _flox = "flox [$FLOX_PROMPT_ENVIRONMENTS]"
    else
        set _flox = "$colorBold$_floxPrompt1 $_floxPrompt2$colorReset"
    endif

    # Save the current 'tcsh_prompt' if not already saved.
    if ( ! $?FLOX_SAVE_TCSH_PROMPT ) then
        setenv FLOX_SAVE_TCSH_PROMPT "$prompt"
    endif
    set prompt = "$_flox $FLOX_SAVE_TCSH_PROMPT"

    unset _flox colorReset colorBold colorPrompt1 colorPrompt2 _floxPrompt1 _floxPrompt2
else if ( $?FLOX_SAVE_TCSH_PROMPT ) then
    # Restore the prompt when no environments should be in the prompt
    set prompt = "$FLOX_SAVE_TCSH_PROMPT"
    unsetenv FLOX_SAVE_TCSH_PROMPT
endif

$_flox_activate_tracer $_activate_d/set-prompt.tcsh END
