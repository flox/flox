_nix_bash_completion() {
    local -a words
    local cword cur
    _get_comp_words_by_ref -n ':=&' words cword cur
    unset words[0]
    cword=$((cword - 1 - $OFFSET))
    local have_type
    while IFS= read -r line; do
        local completion=${line%%$'\t'*}
        if [[ -z $have_type ]]; then
            have_type=1
            if [[ $completion == filenames ]]; then
                compopt -o filenames
            else
                if [[ $completion == attrs ]]; then
                    compopt -o nospace
                fi
           fi
        else
            COMPREPLY+=("$completion")
        fi
    done < <(NIX_GET_COMPLETIONS=$cword "${words[@]}")
    __ltrim_colon_completions "$cur"
}
