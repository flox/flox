# A previous in-place deactivation in this shell unsets the tracer, so default
# to a no-op rather than executing an empty command when sourced again.
if not set -q _flox_activate_tracer; or test -z "$_flox_activate_tracer"
    set -g _flox_activate_tracer true
end

"$_flox_activate_tracer" "$_activate_d/set-prompt.fish" START

if set -q FLOX_PROMPT_ENVIRONMENTS && test -n "$FLOX_PROMPT_ENVIRONMENTS"
    if not set -q FLOX_PROMPT
        set FLOX_PROMPT flox
    end

    if set -q NO_COLOR
        set _flox "flox [$FLOX_PROMPT_ENVIRONMENTS]"
    else
        set colorPrompt1 \e\[38\;5\;$FLOX_PROMPT_COLOR_1""m
        set colorPrompt2 \e\[38\;5\;$FLOX_PROMPT_COLOR_2""m
        set _floxPrompt1 $colorPrompt1$FLOX_PROMPT
        set _floxPrompt2 $colorPrompt2"["$FLOX_PROMPT_ENVIRONMENTS"]"
        set _flox (set_color --bold)$_floxPrompt1" "$_floxPrompt2
    end

    if not functions -q flox_saved_fish_prompt
        functions --copy fish_prompt flox_saved_fish_prompt
    end

    function fish_prompt
        set -l original_prompt (flox_saved_fish_prompt | string collect --no-trim-newlines)
        printf "%s %s\n" $_flox $original_prompt
    end
else if functions -q flox_saved_fish_prompt
    # `functions --copy SRC DST` requires DST to not exist, so erase first
    functions --erase fish_prompt
    functions --copy flox_saved_fish_prompt fish_prompt
    functions --erase flox_saved_fish_prompt
end

"$_flox_activate_tracer" "$_activate_d/set-prompt.fish" END
