# When a project is activated with --dir check
# - hello is installed
# - prompt is set ### XXX how exactly? I don't see it doing this.
# Assume throughout that the project is named project-\d+
set -euxo pipefail

# check for hello
[[ "$({ command -v hello||which hello||type -P hello; } 2>&1)" =~ bin\/hello ]]

# check for hello after changing directory
cd ..
[[ "$({ command -v hello||which hello||type -P hello; } 2>&1)" =~ bin\/hello ]]
