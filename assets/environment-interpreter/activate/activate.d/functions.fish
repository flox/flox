# shellcheck shell=fish

# source_once <path>
# Sources specified file only once per shell invocation.
function source_once
  set -l _guard_path $argv[1]
  # normalize → underscores, collapse runs, trim edges
  set -l _guard_id (string replace --all --regex '[^0-9A-Za-z]' '_' -- $_guard_path)
  set _guard_id (string replace --regex '_+' '_' -- $_guard_id)
  set _guard_id (string trim --chars '_' -- $_guard_id)

  set -l _guard_var "__guard_$_guard_id"

  # test existence; fish's 'set -q' returns 0 if var exists
  if not set -q $_guard_var
    # define globally (not exported)
    eval set -g "$_guard_var" 1
    source "$_guard_path"
  end
end
