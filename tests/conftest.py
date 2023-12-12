import pytest

@pytest.fixture
def flox(tmp_path):
    """
    """

    # flox install pip python3
    # assert_success;
    # assert_output --partial "✅ 'pip' installed to environment";
    # assert_output --partial "✅ 'python3' installed to environment";

    # flox activate
    # which python3
    # which pip
    # cat $PIP_CONFIG_FILE
    #    require-virtualenv = true

    # pip install requests
    #    ERROR: Could not find an activated virtualenv


    # python3 -m venv env
    # ./env/bin/pip install requests
    # ./env/bin/python -c 'import requests; print(requests.__path__)'
    #   \\\['.*/project-\\d+/env/lib/python\\d+.\\d+/site-packages/requests'\\\]

