"$_flox_activate_tracer" "$_activate_d/set-prompt.fish" START

if set -q FLOX_PROMPT_ENVIRONMENTS && test -n "$FLOX_PROMPT_ENVIRONMENTS" && [ "$_FLOX_SET_PROMPT" != false ]
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
end

"$_flox_activate_tracer" "$_activate_d/set-prompt.fish" END
