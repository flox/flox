# Error

```
# Writing manifest to image destination
# time="2024-12-17T13:17:59-07:00" level=debug msg="DoRequest Method: GET URI: http://d/v5.2.3/libpod/images/ghcr.io%2Fflox%2Fflox:v1.3.8/json"
# time="2024-12-17T13:17:59-07:00" level=debug msg="DoRequest Method: POST URI: http://d/v5.2.3/libpod/containers/create"
# Error: statfs /tmp/home.D4hkdr: no such file or directory
# time="2024-12-17T13:17:59-07:00" level=debug msg="Shutting down engines"
# Error: unable to load image: payload does not match any of the supported image formats:
#  * oci: open /var/tmp/libpod-images-load.tar2221354042/index.json: not a directory
#  * oci-archive: loading index: open /var/tmp/container_images_oci2006874804/index.json: no such file or directory
#  * docker-archive: loading tar component "manifest.json": file does not exist
#  * dir: open /var/tmp/libpod-images-load.tar2221354042/manifest.json: not a directory
# ❌ ERROR: Writing to runtime was unsuccessful
```

Matthew says that he sees this same error when `flox` fails to build.

- I verified that `flox` builds my itself in this container.
- I verified that the tmp home directory `/tmp/home.XXXXXX` exists outside
  the container.

Confirmed with Matthew that it's just the second part of the error that he sees,
which suggests that the home directory isn't being mounted properly.

Stopped the podman machine outside of the tests so that tests will restart it.
Same issue.

Full command:
```
/nix/store/p3lz44rirshy6z1f3mzdgk2py9mqssjx-podman-5.2.3/bin/podman run --rm --userns keep-id --log-level debug --mount type=bind,source=/private/tmp/nix-shell.tvmXfz/bats-run-wzT5Ib/test/1/test,target=/flox_env --env HOME=/flox_home --mount type=bind,source=/tmp/home.SoDC1j,target=/flox_home --env FLOX_DISABLE_METRICS=true ghcr.io/flox/flox:v1.3.8 nix --extra-experimental-features nix-command flakes run github:flox/flox/v1.3.8 -- containerize -vvv --dir /flox_env --tag latest --file -
```

Debugging command:
```
/nix/store/p3lz44rirshy6z1f3mzdgk2py9mqssjx-podman-5.2.3/bin/podman run --rm --userns keep-id --log-level debug --mount type=bind,source=/Users/zmitchell/src/flox/macos-containerize-ci/my_env,target=/flox_env --env HOME=/flox_home --mount type=bind,source=/tmp/home.SoDC1j,target=/flox_home --env FLOX_DISABLE_METRICS=true ghcr.io/flox/flox:v1.3.8 nix --extra-experimental-features nix-command flakes run github:flox/flox/v1.3.8 -- containerize -vvv --dir /flox_env --tag latest --file img.tar.gz
```

I'm now at an error regarding users and permissions:
```
DEBU[0029] DoRequest Method: POST URI: http://d/v5.2.3/libpod/containers/6154375310ac7a6dac31174cfd60524e52cbd80f44f5ed2a2f4b01b5dc49e802/start
error: could not set permissions on '/nix/var/nix/profiles/per-user' to 755: Operation not permitted
```

There are these different possible users:
- user on my host machine
- user for the `podman` invocation in the test
- user in the VM
- user in the container

Running on macOS via `just integ-tests` all four of those seem to be `zmitchell`.
I tried setting the user in the `podman` wrapper to `zmitchell`, but that didn't do
anything (as expected) because the user was already set to `zmitchell` everywhere.

I ran this command to put me inside the container:
```
USER=podman-user /nix/store/p3lz44rirshy6z1f3mzdgk2py9mqssjx-podman-5.2.3/bin/podman run -it --rm --userns keep-id --log-level debug --mount type=bind,source=/Users/zmitchell/src/flox/macos-containerize-ci/my_env,target=/flox_env --env HOME=/flox_home --mount type=bind,source=/tmp/home.SoDC1j,target=/flox_home --env FLOX_DISABLE_METRICS=true ghcr.io/flox/flox:v1.3.8 bash
```

Observations:
- `USER` inside the container is `root`.
- `id` does not say that my user ID is the same as `root`
  - `uid=501(core) gid=1000(core) groups=1000(core)`
- `/nix` and everything under it is owned by root.

Noticing that my user ID was the same, but the user itself wasn't, I decided
to change the `--userns` mapping from `keep-id` to `''`, which makes the user
inside the container `root`.
I had to delete the `/flox_home/.cache/nix` directory because it's a symlink
to the host machine that I don't think gets propagated to the container.

`flox containerize` fails with no other errors other than "could not build container".
I switched from `github:flox/flox/v1.3.8` to `github:flox/flox` and I can now
run `flox containerize`.

# Running one machine for the whole file

Doesn't work right now, probably because sockets aren't where podman expects.

Looks like `nix develop` does this at startup:
- Create a temporary directory with template `nix-shell.XXXXXX` under the
  default temporary directory (`/tmp`).
- Sets `"TMP", "TMPDIR", "TEMP", "TEMPDIR"` to point to this directory.

That explains why you see a bunch of paths that start with `/tmp/nix-shell.XXXXXX`
when debugging things.
`podman` appears to create a temporary directory to put runtime data in,
rooted at `TEMPDIR/podman`.

`podman system connection list` reports sockets at `~/.local/share/containers/podman/machine/machine`.
That means we probably need to symlink `XDG_DATA_HOME` into the temporary home
directory.

The current failure is 
```
# Error: unable to connect to Podman socket: Get "http://d/v5.2.3/libpod/_ping":
dial unix /tmp/nix-shell.tvmXfz/storage-run-501/podman/podman.sock: connect: no such file or directory
```

It looks like `skopeo` creates the `storage-run-*` directory:
- If `XDG_RUNTIME_DIR` is set, put it there.
- If not, put it under `TEMPDIR` (which is set under `/tmp/nix-shell.XXXXXX`)
  for us.

When starting a machine per test, the running processes and their files are:
- gvproxy
  - `<nix-shell-tmpdir>/podman/podman-machine-default-gvproxy.sock`
  - `<nix-shell-tmpdir>/podman/podman-machine-default-api.sock`
  - `/run/user/501/podman/podman.sock`
  - `<podman-global-dir>/.local/share/containers/podman/machine/machine`
  - `<nix-shell-tmpdir>/podman/gvproxy.pid`
  - `<nix-shell-tmpdir>/podman/gvproxy.log`
- vfkit
  - `<podman-global-dir>/.local/share/containers/podman/machine/applehv/efi-bl-podman-machine-default`
  - `<podman-global-dir>/.local/share/containers/podman/machine/applehv/podman-machine-default-arm64.raw`
  - `<nix-shell-tmpdir>/podman/podman-machine-default.sock`
  - `<nix-shell-tmpdir>/podman/podman-machine-default.log`
  - `<nix-shell-tmpdir>/podman/podman-machine-default-gvproxy.sock`
  - `<podman-global-dir>/.local/share/containers/podman/machine/applehv/podman-machine-default-ignition.sock`
  
Some of these should be taken care of by the test helpers.
To refresh my own memory, this is the setup procedure:
- setup_file
  - podman_global_dirs_setup
    - make a tempdir with a short path: /tmp/XXXXXX
    - export XDG_CONFIG_HOME, XDG_DATA_HOME, and XDG_RUNTIME_DIR under this path
    - create the corresponding directories
    - export PODMAN_* vars pointing at specific directories under the corresponding XDG dirs
    - create the corresponding directories
    - write a policy file
- setup
  - podman_home_setup
    - make /tmp/home.XXXXXX
    - FLOX_TEST_HOME=/tmp/home.XXXXXX
    - podman_xdg_vars_setup
      - xdg_reals_setup (setup_suite.sh)
        - Set XDG vars to concrete paths if they aren't set
        - export these under REAL_*
        - unset all XDG vars
        - **aha! This unsets the global podman vars!**
      - set XDG_CACHE_HOME to /tmp/home.XXXXXX/.cache
      - set XDG_STATE_HOME to /tmp/home.XXXXXX/.local/.state
      - Create those directories and make them writable for the test user
      - export XDG_CACHE_HOME and XDG_STATE_HOME

I fixed that, and set up a machine-per-file test run.
I'm getting the "connection reset by peer" issue again.
I looked at the `gvproxy` logs and it looks like it's either failing to start
or timing out.
I found this issue which is related:
https://github.com/containers/gvisor-tap-vsock/issues/303

It turned out I was also ignoring errors starting the podman machine.
Now I see this error:
```
# time="2025-01-07T13:13:11-07:00" level=info msg="Using unix socket /tmp/EAKTCD/podman/podman-machine-default-gvproxy.sock"
# Error: dial unixgram /tmp/nix-shell.wt90fh/bats-run-vdOKtf/suite/home/Library/Application Support/vfkit/net-1573-1697598649.sock->/tmp/EAKTCD/podman/podman-machine-default-gvproxy.sock: bind: invalid argument
```

It looks like it's against the law to `bind` (specifically `bind`, not connect)
to a symbolic link to a UDS.
The line that creates the offending socket is here:
https://github.com/crc-org/vfkit/blob/441c13483b5d5a82f8053ab011f879559df9e7c1/pkg/vf/virtionet.go#L31

And it looks like the `Application Support` path is hardcoded i.e. it's not
the macOS equivalent to some XDG directory:
https://github.com/crc-org/vfkit/blob/441c13483b5d5a82f8053ab011f879559df9e7c1/pkg/vf/virtionet.go#L27

Since this is stored in the home directory, and we only create the shorter-path
home directory _after_ the machine is started, it may mean that we need to use
a single home directory and share it with all of the tests.
