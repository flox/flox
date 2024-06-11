set -euxo pipefail

[[ "$({ command -v python3||which python3||type -P python3; } 2>&1)" =~ \.flox/run/.*\.project-.*/bin/python3 ]]

[[ "$({ command -v pip||which pip||type -P pip; } 2>&1)" =~ \.flox/run/.*\.project-.*/bin/pip ]]


[[ "$(cat "$PIP_CONFIG_FILE")" =~ "require-virtualenv = true" ]]

if PIP_OUTPUT="$(pip install requests 2>&1)" || [[ "$PIP_OUTPUT" != "ERROR: Could not find an activated virtualenv (required)." ]]; then
    echo "pip install requests should fail without an activated virtualenv"
    exit 1
fi

python3 -m venv env

# create python virtualenv
./env/bin/pip install requests

[[ "$(./env/bin/python -c 'import requests; print(requests.__path__)')" =~ project-.*/env/lib/python.*/site-packages/requests ]]
