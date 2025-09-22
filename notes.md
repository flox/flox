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
