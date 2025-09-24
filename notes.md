expected:
  assert_equal "${lines[0]}" "Sourcing .profile"
  assert_equal "${lines[1]}" "Setting PATH from .profile"
  assert_equal "${lines[2]}" "sourcing hook.on-activate"
  assert_equal "${lines[3]}" "Sourcing .bashrc"
  assert_equal "${lines[4]}" "Setting PATH from .bashrc"
  assert_equal "${lines[5]}" "sourcing profile.common"
  assert_equal "${lines[6]}" "sourcing profile.bash"
found:
# Sourcing .profile
# Setting PATH from .profile
# sourcing hook.on-activate
# sourcing profile.common
# sourcing profile.bash

So we don't see the lines that come from sourcing .bashrc
But was it already sourced?

implicit setup
setup_file
  common_file_setup
setup
  common_test_setup
  home_setup
  user_dotfiles_setup
    set KNOWN_PROMPT
    turn off bracketed paste
    set .bashrc
    "Sourcing .bashrc"
    "Setting PATH from .bashrc"
    Set PATH to BADPATH
  setup_isolated_flox

test body
project_setup
  project_setup_common
    create PROJECT_DIR and enter it
  flox init -d PROJECT_DIR
edit the manifest
  Add HELLO_PROFILE_SCRIPT
  Add VARS_HOOK_SCRIPT
in place activation

so what's happening in the test is that since we're doing an in-place activation we never actually source .bashrc in the first place.

# repeat activations in .bashrc doesn't break aliases

Same initial setup as before
flox init -d default
  add bash alias in profile
  default_alias="echo hello default"
flox init -d project
  add bash alias in profile
  project_alias="echo hello project"
add in place activation to .bashrc.extra, which will load after .bashrc

Right now it's failing because it's not getting the correct prompt.
Inside the expect command the PS1 variable is getting populated with: `flox [project] \s-\v\$\r\n`
PS1 is no longer `KNOWN_PROMPT` by the time we go to set the flox prompt
It's correct until we activate the project environment

start bash
loads .bashrc
  loads .bashrc.extra
    eval $(flox activate -d default)
      ??? loads .bashrc again or not ???
eval $(flox activate -d default) (again)
  ??? does what ???
expect
  flox activate -d project
    ??? does what ???


- Not loading *my* .bashrc

Creating a new shell
load .bashrc
  no variables set
  do flox activate in place

It looks like we set the shell prompt to the same value twice?

# If you wanted to rewrite the prompt code...
```
flox-activations set-prompt
--shell bash
--shopt "$(shopt)"
--no-color "${NO_COLOR:-empty}"
--ps1 "${PS1:-empty}"
--set-prompt "${_FLOX_SET_PROMPT:-empty}"
```

# new failures

not ok 126 bash: confirm hooks and dotfiles sourced correctly in 578ms
not ok 140 in-place activate works with bash 3 in 632ms
not ok 206 interactive: bash attachs to an activation from the previous release in 1033ms
not ok 212 in-place: bash attachs to an activation from the previous release in 892ms
not ok 237 bash: in-place: nested activation repairs (MAN)PATH in 607ms

## Confirm hooks and dotfiles sourced correctly


