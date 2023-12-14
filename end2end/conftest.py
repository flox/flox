import contextlib
import dataclasses
import os
import pathlib
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import textwrap

import pytest
import pexpect


DEFAULT_TIMEOUT = 3  # in seconds

@pytest.fixture(scope="function")
def home_path(tmp_path_factory):
    """Path to home directory"""
    return tmp_path_factory.mktemp("home")


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
        kwargs.setdefault("capture_output", True)
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        kwargs.setdefault("check", False)
        kwargs.setdefault("text", True)
        kwargs.setdefault("shell", False)

        return subprocess.run(args, **kwargs)

    return _run


@pytest.fixture
def spawn(request, home_path, flox_env):
    """Spawn a command"""

    @contextlib.contextmanager
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
        env = {}
        env.update({
            "PS1": prompt,
            "SHELL": shell_command,
            "PATH": os.environ["PATH"],
            "USER": os.environ["USER"],
            "HOME": os.environ["HOME"],
        })

        kwargs.setdefault("encoding", "utf-8")
        kwargs["env"]=env
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)   # timeout in seconds
        # The (height, width) of the TTY commands run in. 24 is the default.
        # The width needs to be larger than the longest command, as
        # otherwise the command string gets truncated and the shell.expect
        # calls fail to match the the pattern's full command against the
        # truncated output.
        kwargs.setdefault("dimensions", (24, 10000))
        kwargs.setdefault("cwd", home_path)

        with pathlib.Path(kwargs["cwd"]):

            shell = pexpect.spawn(
                shell_command,
                args=shell_command_args,
                **kwargs,
            )

            old_expect = shell.expect

            def new_expect(*args, **kwargs):
                try:
                    old_expect(*args, **kwargs)
                except Exception as e:
                    print("=" * 80)
                    print("= DEBUG HELP:")
                    print("=" * 80)
                    print("EXPECT:")
                    print(textwrap.indent(str(shell), " " * 2))
                    print(str(flox_env))
                    print("=" * 80)
                    raise e

            def send_command(cmd):
                shell.sendline(cmd)
                shell.expect_exact(cmd + "\r\n")

            def expect_prompt(timeout=DEFAULT_TIMEOUT):
                shell.expect(
                    r"{prompt}".format(prompt=re.escape(prompt)),
                    timeout=timeout,
                )

            # helper methods
            shell.expect = new_expect
            shell.send_command = send_command
            shell.expect_prompt = expect_prompt

            if request.config.getoption('verbose') >= 2:
                shell.logfile = sys.stdout

            # wait for the prompt
            shell.expect_prompt()

            # send command
            shell.send_command(command)

            yield shell

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
class FloxEnv:
    name: str
    path: pathlib.Path
    run_path: pathlib.Path
    nixpkgs_rev: str

    def __str__(self):
        tmp = "FLOX ENVIRONMENT:\n"
        tmp += (f"  name: {self.name}\n")
        tmp += (f"  path: {self.path}\n")
        tmp += (f"  run_path: {self.run_path}\n")
        tmp += (f"  nixpkgs_rev: {self.nixpkgs_rev}\n")
        return tmp


@pytest.fixture
def flox_env(
        request,
        home_path,
        nix_system,
    ):
    """Everything needed to create a flox environment."""

    project_path = pathlib.Path(tempfile.mkdtemp(
        prefix="flox-tests-environment-",
        dir=home_path,
    ))

    project_name = os.path.basename(project_path)
    nixpkgs_rev = "e8039594435c68eb4f780f3e9bf3972a7399c4b1"

    os.environ["FLOX_DISABLE_METRICS"] = "true"
    os.environ["_PKGDB_GA_REGISTRY_REF_OR_REV"] = nixpkgs_rev
    os.environ["HOME"] = str(home_path)
    os.environ["XDG_CONFIG_HOME"] = str(home_path / ".config")
    os.environ["XDG_CACHE_HOME"] = str(home_path / ".cache")
    os.environ["XDG_DATA_HOME"] = str(home_path / ".local/share")
    os.environ["XDG_STATE_HOME"] = str(home_path / ".local/state")

    (home_path / ".cache").mkdir(parents=True)

    if hasattr(request.config, "cache"):
        # restore flox cache
        flox_cache = request.config.cache.get("flox-cache", None)
        if flox_cache and os.path.exists(flox_cache):
            shutil.copytree(flox_cache, str(home_path / ".cache/flox"), symlinks=True)
        # restore nix cache
        flox_cache = request.config.cache.get("nix-cache", None)
        if flox_cache and os.path.exists(flox_cache):
            shutil.copytree(flox_cache, str(home_path / ".cache/nix"), symlinks=True)

    yield FloxEnv(
        name = project_name,
        path = project_path,
        run_path = project_path / f".flox/run/{nix_system}.{project_name}",
        nixpkgs_rev = nixpkgs_rev,
    )

    if hasattr(request.config, "cache"):
        # save flox cache
        request.config.cache.set("flox-cache", str(home_path / ".cache/flox"))
        # save nix cache
        request.config.cache.set("nix-cache", str(home_path / ".cache/nix"))
