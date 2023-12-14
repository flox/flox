import dataclasses
import os
import pathlib
import re
import shlex
import subprocess
import tempfile

import pytest
import pexpect


DEFAULT_TIMEOUT = 3  # in seconds


@pytest.fixture
def nix():
    """Path to nix binary
    """
    return os.environ.get("NIX_BIN", "nix")


@pytest.fixture
def pkgdb():
    """Path to pkgdb binary
    """
    return os.environ.get("PKGDB_BIN", "pkgdb")


@pytest.fixture
def flox():
    """Path to flox binary
    """
    return os.environ.get("FLOX_BIN", "flox")

@pytest.fixture
def run():
    """Run a command"""

    def _run(args, **kwargs):

        # Split into list of arguments when string is provided
        if isinstance(args, str):
            args = shlex.split(args)

        # Our default subprocess.run arguments, see docs for more:
        #   -> https://docs.python.org/3/library/subprocess.html
        kwargs.setdefault("capture_output", True)       # stdout and stderr will be captured
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)   # timeout in seconds
        kwargs.setdefault("check", False)               # don't raise any expection
        kwargs.setdefault("text", True)
        kwargs.setdefault("shell", False)

        return subprocess.run(args, **kwargs)
    return _run


@pytest.fixture
def spawn():
    """Spawn a command"""

    def _run(command, **kwargs):

        # Join list of arguments into string
        if isinstance(command, list):
            command = " ".join(command)

        # TODO: support configuring default prompt in the future.
        prompt = "$ "

        # TODO: support configuring other shell than bash in the future.
        shell_command = "bash"
        shell_command_args = ["--norc", "--noprofile"]

        # Pass on every environment variable, but set $PS1 and $SHELL
        env = os.environ.copy()
        env.update({
            "PS1": prompt,
            "SHELL": shell_command,
        })

        kwargs.setdefault("encoding", "utf-8")
        kwargs.setdefault("env", env)
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)   # timeout in seconds
        # The (height, width) of the TTY commands run in. 24 is the default.
        # The width needs to be larger than the longest command, as
        # otherwise the command string gets truncated and the shell.expect
        # calls fail to match the the pattern's full command against the
        # truncated output.
        kwargs.setdefault("dimensions", (24, 10000))

        shell = pexpect.spawn(
            shell_command,
            args=shell_command_args,
            **kwargs,
        )

        def send_command(cmd):
            shell.sendline(cmd)
            shell.expect_exact(cmd + "\r\n")

        def expect_prompt(timeout=DEFAULT_TIMEOUT):
            shell.expect(r"{prompt}".format(prompt=re.escape(prompt)), timeout=timeout)

        # helper methods
        shell.prompt = prompt
        shell.send_command = send_command
        shell.expect_prompt = expect_prompt

        # wait for the prompt
        shell.expect_prompt()

        # send command
        shell.send_command(command)

        return shell
    return _run


@pytest.fixture
def nix_system(run, nix):
    """Current nix system"""
    res = run(
        [
            nix,
            "--experimental-features", "nix-command",
            "eval", "--impure", "--raw", "--expr", "builtins.currentSystem",
        ],
        check=True,
    )
    return res.stdout.strip()


@dataclasses.dataclass
class FloxProject:
    name: str
    path: pathlib.Path
    run_path: pathlib.Path
    nixpkgs_rev: str


@pytest.fixture
def flox_project(
        tmp_path,
        nix_system,
    ):
    """Path to flox project"""

    project_path = pathlib.Path(tempfile.mkdtemp(
        prefix="flox-tests-envs-",
        dir=tmp_path,
    ))

    project_name = os.path.basename(project_path)
    nixpkgs_rev = "e8039594435c68eb4f780f3e9bf3972a7399c4b1"

    os.environ["FLOX_DISABLE_METRICS"] = "true"
    os.environ["_PKGDB_GA_REGISTRY_REF_OR_REV"] = nixpkgs_rev
    os.environ["HOME"] = str(tmp_path)
    os.environ["XDG_CONFIG_HOME"] = str(tmp_path / ".config")
    os.environ["XDG_CACHE_HOME"] = str(tmp_path / ".cache")
    os.environ["XDG_DATA_HOME"] = str(tmp_path / ".local/share")
    os.environ["XDG_STATE_HOME"] = str(tmp_path / ".local/state")

    return FloxProject(
        name = project_name,
        path = project_path,
        run_path = project_path / f".flox/run/{nix_system}.{project_name}",
        nixpkgs_rev = nixpkgs_rev,
    )
