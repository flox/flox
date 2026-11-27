# rc file for hook-prompt-callback.exp: the user's own prompt callback reports
# the exit status it was run with, and the hook is installed by evaluating
# in-place activation twice. Activation builds the prompt from the PS1 it
# finds, or from FLOX_SAVE_BASH_PS1 when a surrounding Flox shell exported one.
unset FLOX_SAVE_BASH_PS1
PS1='callback> '
user_prompt() { printf 'USER:%s\n' "$?"; }
PROMPT_COMMAND=user_prompt
eval "$("$FLOX_BIN" activate -d "$PROJECT_DIR")"
eval "$("$FLOX_BIN" activate -d "$PROJECT_DIR")"
