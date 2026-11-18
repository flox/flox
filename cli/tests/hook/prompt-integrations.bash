# shellcheck shell=bash
#
# The PROMPT_COMMAND handling of two prompt integrations, reduced to what
# decides whether Flox's `_flox_hook` registration finds an earlier one.
#
# bash-preexec:
#   https://github.com/rcaloras/bash-preexec/blob/5ae4758c36e8391fb3932e6ae68c283489fc813d/bash-preexec.sh
#   `__bp_trim_whitespace`, `__bp_sanitize_string`,
#   `__bp_use_array_prompt_command`, `__bp_remove_command_from_prompt_command`
#   and `__bp_install_prompt_command` are copied with their comments removed.
#   Upstream defers `__bp_install` to the first prompt by adding
#   `__bp_install "$_"` to PROMPT_COMMAND; it then removes that string, sets up
#   the DEBUG trap and the precmd/preexec arrays, and calls
#   `__bp_install_prompt_command`. Here the test calls `__bp_install` directly
#   and it only makes that last call. `__bp_precmd_invoke_cmd` also calls
#   `__bp_install_prompt_command` at every prompt, before it runs the precmd
#   functions, and is reduced to that call. None of the left-out steps moves
#   `_flox_hook` within PROMPT_COMMAND.
#
# VS Code shell integration:
#   https://github.com/microsoft/vscode/blob/776b518e720e780fb590f29872473f90bda9e079/src/vs/workbench/contrib/terminal/common/scripts/shellIntegration-bash.sh
#   `__vsc_restore_exit_code`, `__vsc_prompt_cmd_original` and
#   `__vsc_prompt_cmd` are copied unchanged, and the top-level block that
#   installs them is wrapped in `__vsc_install`; `__vsc_precmd`, which emits
#   terminal escape sequences, is a no-op. As upstream, `__vsc_install` leaves
#   PROMPT_COMMAND alone when bash-preexec is loaded (`bash_preexec_imported`
#   set), since VS Code then hooks in through bash-preexec's precmd functions.
#
# bash-preexec license (LICENSE.md at the commit above):
#
#   The MIT License
#
#   Copyright (c) 2017 Ryan Caloras and contributors (see https://github.com/rcaloras/bash-preexec)
#
#   Permission is hereby granted, free of charge, to any person obtaining a copy
#   of this software and associated documentation files (the "Software"), to deal
#   in the Software without restriction, including without limitation the rights
#   to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
#   copies of the Software, and to permit persons to whom the Software is
#   furnished to do so, subject to the following conditions:
#
#   The above copyright notice and this permission notice shall be included in
#   all copies or substantial portions of the Software.
#
#   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
#   IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
#   FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
#   AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
#   LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
#   OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
#   THE SOFTWARE.
#
# VS Code license (LICENSE.txt at the commit above):
#
#   MIT License
#
#   Copyright (c) 2015 - present Microsoft Corporation
#
#   Permission is hereby granted, free of charge, to any person obtaining a copy
#   of this software and associated documentation files (the "Software"), to deal
#   in the Software without restriction, including without limitation the rights
#   to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
#   copies of the Software, and to permit persons to whom the Software is
#   furnished to do so, subject to the following conditions:
#
#   The above copyright notice and this permission notice shall be included in all
#   copies or substantial portions of the Software.
#
#   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
#   IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
#   FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
#   AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
#   LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
#   OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
#   SOFTWARE.

# ---------------------------------------------------------------------------- #
# bash-preexec

__bp_trim_whitespace() {
    local var=${1:?} text=${2:-}
    text="${text#"${text%%[![:space:]]*}"}"   # remove leading whitespace characters
    text="${text%"${text##*[![:space:]]}"}"   # remove trailing whitespace characters
    printf -v "$var" '%s' "$text"
}

__bp_sanitize_string() {
    local var=${1:?} sanitized=${2:-}

    local unset_extglob=
    if ! shopt -q extglob; then
        unset_extglob=yes
        shopt -s extglob
    fi

    local tmp nl=$'\n'
    while
        tmp="${sanitized//?(+([[:blank:]]))[";$nl"]*([[:blank:]]):*([[:blank:]])[";$nl"]*([[:blank:]])/$nl}"
        [[ "$tmp" != "$sanitized" ]]
    do
        sanitized="$tmp"
    done
    sanitized="${sanitized#:*([[:blank:]])[";$nl"]}"
    sanitized="${sanitized%[";$nl"]*([[:blank:]]):}"
    __bp_trim_whitespace sanitized "$sanitized"
    sanitized=${sanitized%;}
    sanitized=${sanitized#;}
    __bp_trim_whitespace sanitized "$sanitized"
    if [[ "$sanitized" == ":" ]]; then
        sanitized=
    fi
    printf -v "$var" '%s' "$sanitized"

    if [[ -n "$unset_extglob" ]]; then
        shopt -u extglob
    fi
}

__bp_use_array_prompt_command() {
    (( BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1) ))
}

__bp_remove_command_from_prompt_command() {
    local removed_command="${1-}"
    if __bp_use_array_prompt_command; then
        local i sanitized_prompt_command
        for i in "${!PROMPT_COMMAND[@]}"; do
            sanitized_prompt_command="${PROMPT_COMMAND[i]:-}"
            sanitized_prompt_command="${sanitized_prompt_command//"$removed_command"/:}"
            __bp_sanitize_string sanitized_prompt_command "$sanitized_prompt_command"
            if [[ -n "$sanitized_prompt_command" ]]; then
                PROMPT_COMMAND[i]="$sanitized_prompt_command"
            else
                unset -v 'PROMPT_COMMAND[i]'
            fi
        done
    else
        local sanitized_prompt_command="${PROMPT_COMMAND:-}"
        sanitized_prompt_command="${sanitized_prompt_command//"$removed_command"/:}" # no-op
        __bp_sanitize_string PROMPT_COMMAND "$sanitized_prompt_command"
    fi
}

__bp_install_prompt_command() {
    local prompt_command="${PROMPT_COMMAND:-}"
    if __bp_use_array_prompt_command; then
        local IFS=$'\n'
        prompt_command="${PROMPT_COMMAND[*]:-}"
        IFS=$' \t\n'
    fi

    # shellcheck disable=SC2016
    local prologue='__bp_precmd_invoke_cmd "$_"'
    local epilogue='__bp_interactive_mode'
    if [[ "$prompt_command" == "$prologue"$'\n'* && "$prompt_command" == *$'\n'"$epilogue" ]]; then
        return 1
    fi

    __bp_remove_command_from_prompt_command "$prologue"
    __bp_remove_command_from_prompt_command "$epilogue"

    # shellcheck disable=SC2128,SC2178
    PROMPT_COMMAND=$prologue${PROMPT_COMMAND:+$'\n'$PROMPT_COMMAND}
    if __bp_use_array_prompt_command; then
        PROMPT_COMMAND+=("$epilogue")
    else
        # shellcheck disable=SC2179
        PROMPT_COMMAND+=$'\n'$epilogue
    fi
    return 0
}

__bp_install() { __bp_install_prompt_command || true; }
__bp_precmd_invoke_cmd() { __bp_install_prompt_command || true; }
__bp_interactive_mode() { :; }

# ---------------------------------------------------------------------------- #
# VS Code shell integration

__vsc_precmd() { :; }

__vsc_restore_exit_code() {
	return "$1"
}

__vsc_prompt_cmd_original() {
	__vsc_status="$?"
	builtin local cmd
	__vsc_restore_exit_code "${__vsc_status}"
	# Evaluate the original PROMPT_COMMAND similarly to how bash would normally
	# See https://unix.stackexchange.com/a/672843 for technique
	for cmd in "${__vsc_original_prompt_command[@]}"; do
		eval "${cmd:-}"
	done
	__vsc_precmd
}

__vsc_prompt_cmd() {
	__vsc_status="$?"
	__vsc_precmd
}

__vsc_install() {
	__vsc_original_prompt_command=${PROMPT_COMMAND:-}

	if [[ -z "${bash_preexec_imported:-}" ]]; then
		if [[ -n "${__vsc_original_prompt_command:-}" && "${__vsc_original_prompt_command:-}" != "__vsc_prompt_cmd" ]]; then
			PROMPT_COMMAND=__vsc_prompt_cmd_original
		else
			PROMPT_COMMAND=__vsc_prompt_cmd
		fi
	fi
}
