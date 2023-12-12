import os

import pytest


def test_python_integration_with_flox(
        run,
        spawn,
        flox,
        flox_project,
    ):

    # Initialize flox project
    res = run(f"{flox} init")
    assert res.returncode == 0
    # TODO: this should be in stderr
    assert "✨ Created environment" in res.stdout
    assert res.stderr == ""

    assert (flox_project.path / ".flox/env").exists()

    # Install pip and python3 flox project
    res = run(f"{flox} install pip python3", timeout=60)
    assert res.returncode == 0
    assert res.stdout == ""
    assert "✅ 'pip' installed to environment" in res.stderr
    assert "✅ 'python3' installed to environment" in res.stderr

    # TODO: convert spawn into a with-statement context so we are able to do
    #       the following:
    #
    #           with spawn(f"{flox} activate") as shell:
    #               ...
    #
    with flox_project.path:

        # Enter (activate) the flox environment
        shell = spawn(f"{flox} activate")
        shell.expect(r"Building environment...", timeout=10)
        shell.expect_prompt(timeout=30)

        assert (flox_project.run_path / "bin/python3").exists()
        assert (flox_project.run_path / "bin/pip").exists()

        with open(flox_project.path / ".flox/pip.ini") as f:
            assert "require-virtualenv = true" in f.read()

        # check that we configured pip correctly
        shell.send_command("echo $PIP_CONFIG_FILE")
        shell.expect(r"{path}/.flox/pip.ini".format(
            path = flox_project.path,
        ))

        # check that virtualenv is required
        shell.send_command("pip install requests")
        shell.expect(r"ERROR: Could not find an activated virtualenv")
        shell.expect_prompt()

        # create virtualenv
        shell.send_command("python3 -m venv env")
        shell.expect_prompt(timeout=10)

        assert (flox_project.path / "env/bin/python3").exists()
        assert (flox_project.path / "env/bin/pip").exists()

        # install requests library into virtualenv
        shell.send_command("./env/bin/pip install requests")
        shell.expect_prompt(timeout=30)

        # check that we can import requests library
        shell.send_command("./env/bin/python -c 'import requests; print(requests.__path__)'")
        shell.expect(r"{path}/env/lib/python\d+.\d+/site-packages/requests".format(
            path = flox_project.path,
        ))
        shell.expect_prompt()
